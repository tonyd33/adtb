use super::*;

/// The fields of a merged row that tell which merged constructor it came from, with each
/// constructor's values of them as expression text.
struct Disc {
    fields: Vec<String>,
    values: BTreeMap<String, Vec<(String, String)>>,
}

impl Disc {
    fn value(&self, ctor: &str, field: &str) -> Option<&str> {
        self.values
            .get(ctor)?
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, v)| v.as_str())
    }
}

/// A merge's image of the merged constructors: `src_ix` are the indexes of `sources`, in order.
struct MergeImage<'a> {
    def: TypeId,
    ctor: &'a str,
    disc: &'a Disc,
    sources: &'a [String],
    src_ix: &'a [u32],
    carries: &'a Carries,
}

impl Cx<'_> {
    /// With `derived`, the values the routing's arms fix tell the merged constructors apart, and
    /// no field need be a discriminator.
    pub(in crate::evolve) fn merge(
        &mut self,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: Option<&[ast::Arm]>,
        clauses: &[ast::ArmClause],
        derived: bool,
    ) -> R<()> {
        let schema = self.schema()?;
        let (i, t) = self.type_decl(ty)?;
        let def = self.type_id(&schema, ty)?;
        let sources = self.sources(&schema, def, sel)?;
        self.fresh_ctor(&schema, def, into)?;
        let m = into.name.text.clone();
        let m_text = self.ctor_def_text(into);
        let refs = check::schema_refs(&schema);
        let src_ix: Vec<u32> = sources
            .iter()
            .filter_map(|s| schema.ty(def).ctor_index(s))
            .collect();
        let carries = match arms {
            Some(a) => Some(self.routing(
                ty,
                &sources,
                std::slice::from_ref(&m),
                std::slice::from_ref(&m_text),
                a,
            )?),
            None => None,
        };
        let after = self.with_ctors(i, &t, std::slice::from_ref(&m_text))?;
        let ad = self.type_id(&after, ty)?;
        let mc = after.ty(ad).ctor_index(&m).expect("the merged constructor");
        let mfields = after.ty(ad).ctors()[mc as usize].fields.clone();
        let disc = match arms {
            Some(a) if derived => self.fixed_disc(a),
            _ => self.named_disc(&after, &mfields, into, &sources)?,
        };
        let carries: Carries = match (arms, carries) {
            (Some(_), Some(c)) if derived => c,
            (Some(a), Some(c)) => {
                self.own_names(a, &disc.fields[0])?;
                c
            }
            _ => self.carried_by_name(schema.ty(def), &sources, &m, &mfields, &disc)?,
        };
        let tm = self.module_of(ty);
        self.restate(&schema, def, &tm, &sources, std::slice::from_ref(&m), true)?;
        let mut edits = self.reselect(
            &schema,
            def,
            &sources,
            std::slice::from_ref(&m),
            Reselect {
                whole: true,
                clauses: false,
                checks: false,
                what: "the merge",
            },
        )?;
        let image = MergeImage {
            def,
            ctor: &m,
            disc: &disc,
            sources: &sources,
            src_ix: &src_ix,
            carries: &carries,
        };
        edits = Self::merged(edits, self.merged_checks(&schema, def, &refs, &image)?);
        edits = Self::merged(
            edits,
            self.reselect_rows(&schema, def, &sources, std::slice::from_ref(&m), true),
        );
        let clause_edits = self.merged_clauses(&schema, &image)?;
        edits = Self::merged(edits, clause_edits);
        edits = Self::merged(edits, self.replace_ctors(i, &t, &sources, &m_text));
        let arm_edits = self.arm_replacements(&schema, &refs, def, &src_ix, clauses)?;
        edits = Self::merged(edits, arm_edits);
        self.apply_edits(edits);
        self.retype_products(&schema, def, &sources, std::slice::from_ref(&m))
    }

    /// Returns the edits that rescope each clause over the merged constructors to the merged one.
    fn merged_clauses(&self, schema: &Schema, image: &MergeImage) -> R<Edits> {
        let mut clause_edits: Edits = BTreeMap::new();
        let mut added: BTreeMap<usize, Vec<Written>> = BTreeMap::new();
        for cl in self.scoped_clauses(schema, image.def)? {
            let Some(scope) = &cl.scope else { continue };
            let inside = scope.iter().filter(|c| image.src_ix.contains(c)).count();
            if inside == 0 {
                continue;
            }
            let shown = format!("{} ({})", cl.kind, cl.fields.join(", "));
            let text = self.src.decls[cl.decl].text.clone();
            if inside != scope.len() {
                if inside != image.src_ix.len() {
                    return self.err(format!("`{shown}` ranges over some but not all of the merged constructors and has no image under the merge"));
                }
                let td = schema.ty(image.def);
                let mut names: Vec<String> = Vec::new();
                for &c in scope {
                    let n = if image.src_ix.contains(&c) {
                        image.ctor.to_string()
                    } else {
                        td.ctors()[c as usize].name.clone()
                    };
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                let new = format!("{shown} in {}", sel_text(&names));
                clause_edits
                    .entry(cl.decl)
                    .or_default()
                    .push((cl.range.clone(), new.into()));
                continue;
            }
            let out_fields = self.merged_fields(schema, image, &cl, &shown)?;
            clause_edits.entry(cl.decl).or_default().push((
                clause_removal(&text, std::slice::from_ref(&cl.range), 0),
                String::new().into(),
            ));
            let new = format!("{} ({}) in {}", cl.kind, out_fields.join(", "), image.ctor);
            let list = added.entry(cl.decl).or_default();
            if !list.iter().any(|x| x.text == new) {
                list.push(new.into());
            }
        }
        for (k, news) in added {
            let end = self.src.decls[k].text.trim_end().len();
            clause_edits
                .entry(k)
                .or_default()
                .push((end..end, Written::prefixed(news, "\n  ")));
        }
        Ok(clause_edits)
    }

    /// Returns the fields of the merged constructor that clause `cl`'s fields go to, the same from
    /// every constructor it ranges over.
    fn merged_fields(
        &self,
        schema: &Schema,
        image: &MergeImage,
        cl: &ScopedClause,
        shown: &str,
    ) -> R<Vec<String>> {
        let mut out_fields: Option<Vec<String>> = None;
        for &c in cl.scope.iter().flatten() {
            let s = &schema.ty(image.def).ctors()[c as usize].name;
            let mut fs = Vec::new();
            for f in &cl.fields {
                let header = schema.ty(image.def).header().iter().any(|x| x.name == *f);
                let mapped = if header {
                    Some(f.clone())
                } else {
                    image
                        .carries
                        .iter()
                        .find(|(x, y, _, _)| x == s && y == f)
                        .map(|(_, _, _, g)| g.clone())
                };
                match mapped {
                    Some(g) => fs.push(g),
                    None => {
                        return self.err(format!("`{shown}` has no image under the merge: the routing does not carry `{s}.{f}` whole"));
                    }
                }
            }
            if cl.kind == "unique" {
                for f in image.disc.fields.iter().rev() {
                    if !fs.contains(f) {
                        fs.insert(0, f.clone());
                    }
                }
            }
            match &out_fields {
                None => out_fields = Some(fs),
                Some(prev) if *prev == fs => {}
                Some(_) => {
                    return self.err(format!("`{shown}` has no image under the merge: its constructors' fields go to different fields"));
                }
            }
        }
        Ok(out_fields.unwrap_or_default())
    }

    /// Refuse an arm of an author's merge that does not give discriminator `disc` as its own
    /// constructor name.
    fn own_names(&self, arms: &[ast::Arm], disc: &str) -> R<()> {
        for arm in arms {
            let ok = match (&arm.patterns.first().map(|p| &p.kind), &arm.body.kind) {
                (
                    Some(ast::PatternKind::Ctor { path, ctor, .. }),
                    ast::ExprKind::Ctor {
                        record: Some(ast::RecordExpr::Fields(given, _)),
                        ..
                    },
                ) => {
                    let from = &ctor.as_ref().unwrap_or(&path.name).text;
                    given.iter().any(|(g, e)| {
                        g.text == *disc
                            && matches!(&e.kind, ast::ExprKind::Ctor { path: p, ctor: None, record: None } if p.name.text == *from)
                    })
                }
                _ => false,
            };
            if !ok {
                return self.err_at(arm.body.span, format!(
                    "the routing gives the discriminator `{disc}` as each row's own constructor name"
                ));
            }
        }
        Ok(())
    }

    /// Returns what a merge without routing carries: each field of each source to the field of
    /// `m` of the same name and type.
    fn carried_by_name(
        &self,
        td: &schema::TypeDef,
        sources: &[String],
        m: &str,
        mfields: &[schema::FieldDef],
        disc: &Disc,
    ) -> R<Carries> {
        let mut c = Vec::new();
        for s in sources {
            let k = td.ctor_index(s).expect("a source");
            let sf = &td.ctors()[k as usize].fields;
            for f in sf {
                match mfields
                    .iter()
                    .find(|g| g.name == f.name && !disc.fields.contains(&g.name))
                {
                    Some(g) if g.ty == f.ty => {
                        c.push((s.clone(), f.name.clone(), m.to_string(), f.name.clone()));
                    }
                    _ => {
                        return self.err(format!(
                            "field `{s}.{}` has no counterpart in `{m}`",
                            f.name
                        ));
                    }
                }
            }
            for g in mfields {
                if !disc.fields.contains(&g.name) && !sf.iter().any(|f| f.name == g.name) {
                    return self.err(format!(
                        "field `{m}.{}` has no counterpart in `{s}`",
                        g.name
                    ));
                }
            }
        }
        Ok(c)
    }

    /// Returns the discriminator field of merged constructor `into`, typed by a declared nullary sum
    /// whose constructors are exactly `sources`, each source's value its own name.
    fn named_disc(
        &self,
        after: &Schema,
        mfields: &[schema::FieldDef],
        into: &ast::CtorDef,
        sources: &[String],
    ) -> R<Disc> {
        let m = &into.name.text;
        let wanted: BTreeSet<&str> = sources.iter().map(String::as_str).collect();
        let discs: Vec<&schema::FieldDef> = mfields
            .iter()
            .filter(|f| match &f.ty {
                Ty::Data { def: d, args, .. } if args.is_empty() => {
                    let s = after.ty(*d);
                    matches!(s.kind, TypeDefKind::Sum { record: false, .. })
                        && s.ctors().iter().all(|c| c.fields.is_empty())
                        && s.ctors()
                            .iter()
                            .map(|c| c.name.as_str())
                            .collect::<BTreeSet<_>>()
                            == wanted
                }
                _ => false,
            })
            .collect();
        let name = match discs.as_slice() {
            [one] => one.name.clone(),
            [] => {
                return self.err_at(into.name.span, format!(
                    "the merged constructor `{m}` has no discriminator: a field typed by a nullary sum whose constructors are exactly {}",
                    sources.join(", ")
                ));
            }
            _ => {
                return self.err_at(
                    into.name.span,
                    format!("the merged constructor `{m}` has more than one discriminator field"),
                );
            }
        };
        Ok(Disc {
            fields: vec![name.clone()],
            values: sources
                .iter()
                .map(|s| (s.clone(), vec![(name.clone(), s.clone())]))
                .collect(),
        })
    }

    /// Returns the fields the arms of a derived merge give a value not read from the row, with
    /// each source's values.
    fn fixed_disc(&self, arms: &[ast::Arm]) -> Disc {
        let mut disc = Disc {
            fields: Vec::new(),
            values: BTreeMap::new(),
        };
        for a in arms {
            let (
                Some(ast::PatternKind::Ctor { path, ctor, .. }),
                ast::ExprKind::Ctor {
                    record: Some(ast::RecordExpr::Fields(given, _)),
                    ..
                },
            ) = (a.patterns.first().map(|p| &p.kind), &a.body.kind)
            else {
                continue;
            };
            let from = ctor.as_ref().unwrap_or(&path.name).text.clone();
            for (g, e) in given {
                if matches!(e.kind, ast::ExprKind::Field(..) | ast::ExprKind::Name(_)) {
                    continue;
                }
                if !disc.fields.contains(&g.text) {
                    disc.fields.push(g.text.clone());
                }
                disc.values
                    .entry(from.clone())
                    .or_default()
                    .push((g.text.clone(), self.text[e.span.range()].to_string()));
            }
        }
        disc
    }

    /// Returns the edits that carry each check over the merged constructors to the merged one. A check
    /// over some but not all of them becomes a check matching on the discriminator, with `true` for the
    /// others.
    fn merged_checks(
        &self,
        schema: &Schema,
        def: TypeId,
        refs: &[(FileId, Span, Ref)],
        image: &MergeImage,
    ) -> R<Edits> {
        let td = schema.ty(def);
        let rels: Vec<RelId> = Self::relations_of(schema, def)
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        let sets: BTreeSet<&str> = rels
            .iter()
            .flat_map(|&r| schema.relation(r).sets.iter().map(|x| x.name.as_str()))
            .collect();
        let mut edits: Edits = BTreeMap::new();
        for chk in &schema.checks {
            let Some(cs) = &chk.ctors else { continue };
            if !rels.contains(&chk.relation) {
                continue;
            }
            let inside = cs.iter().filter(|c| image.src_ix.contains(c)).count();
            if inside == 0 {
                continue;
            }
            let partial = inside < image.src_ix.len();
            if partial && inside < cs.len() {
                return self.err(format!(
                    "check `{}` has no image under the merge; drop or rewrite it in this or an earlier migration",
                    chk.name
                ));
            }
            let j = chk.file.0 as usize;
            let Some(Decl::Check(cd)) = self.src.parsed(j) else {
                continue;
            };
            let Some(sel) = &cd.on.sel else { continue };
            let pred = self.src.local(j, cd.predicate.span);
            let body = self.carried_predicate(td, refs, image, chk, j, pred.clone())?;
            let idents: Vec<ast::Ident> = sel_idents(sel).into_iter().cloned().collect();
            let entry = edits.entry(j).or_default();
            if !idents.iter().any(|x| sets.contains(x.text.as_str())) {
                let mut names: Vec<String> = Vec::new();
                for &c in cs {
                    let n = if image.src_ix.contains(&c) {
                        image.ctor.to_string()
                    } else {
                        td.ctors()[c as usize].name.clone()
                    };
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                entry.push((self.sel_range(j, &idents), sel_text(&names).into()));
            }
            if !partial {
                entry.push((pred, body.into()));
                continue;
            }
            let mut nested = false;
            crate::visit::each_expr(&cd.predicate, &mut |e| {
                nested |= matches!(e.kind, ast::ExprKind::Match(..));
            });
            let body = if nested { format!("({body})") } else { body };
            let untold = || {
                self.err(format!(
                    "check `{}` has no image under the merge: no one field of `{}` tells its constructors apart",
                    chk.name, image.ctor
                ))
            };
            let [field] = image.disc.fields.as_slice() else {
                return untold();
            };
            let mut arms: Vec<String> = Vec::new();
            for (s, c) in image.sources.iter().zip(image.src_ix) {
                let Some(v) = image.disc.value(s, field) else {
                    return untold();
                };
                let e = if cs.contains(c) {
                    body.as_str()
                } else {
                    "true"
                };
                arms.push(format!("{v} -> {e}"));
            }
            entry.push((pred, format!("match {field} | {}", arms.join(" | ")).into()));
        }
        Ok(edits)
    }

    /// Returns the text at `pred` of check declaration `j` with each field of a merged constructor
    /// renamed to the field of the merged constructor the routing carries it to whole.
    fn carried_predicate(
        &self,
        td: &schema::TypeDef,
        refs: &[(FileId, Span, Ref)],
        image: &MergeImage,
        chk: &schema::CheckDef,
        j: usize,
        pred: Range<usize>,
    ) -> R<String> {
        let refused = |why: String| {
            self.err(format!(
                "check `{}` has no image under the merge: {why}",
                chk.name
            ))
        };
        let cs = chk.ctors.as_deref().unwrap_or_default();
        let mut renames: Vec<(Range<usize>, String)> = Vec::new();
        for (file, span, r) in refs {
            let Ref::Field {
                def: d, name, pun, ..
            } = r
            else {
                continue;
            };
            if *file != chk.file || td.header().iter().any(|h| h.name == *name) {
                continue;
            }
            let at = self.src.local(j, *span);
            if !(pred.start <= at.start && at.end <= pred.end) || *d != image.def {
                continue;
            }
            let mut to: Option<&str> = None;
            for &c in cs {
                let s = &td.ctors()[c as usize].name;
                let carried = if image.src_ix.contains(&c) {
                    image
                        .carries
                        .iter()
                        .find(|(x, y, _, _)| x == s && y == name)
                        .map(|(_, _, _, g)| g.as_str())
                } else {
                    Some(name.as_str())
                };
                match (carried, to) {
                    (None, _) => {
                        return refused(format!("the routing does not carry `{s}.{name}` whole"));
                    }
                    (Some(g), Some(prev)) if g != prev => {
                        return refused(format!(
                            "its constructors' field `{name}` goes to different fields"
                        ));
                    }
                    (Some(g), _) => to = Some(g),
                }
            }
            if let Some(to) = to
                && to != name
            {
                let text = if *pun {
                    format!("{name}: {to}")
                } else {
                    to.to_string()
                };
                renames.push((at.start - pred.start..at.end - pred.start, text));
            }
        }
        renames.sort_by_key(|(r, _)| r.start);
        let old = &self.src.decls[j].text[pred];
        let mut body = String::new();
        let mut from = 0;
        for (r, text) in &renames {
            body.push_str(&old[from..r.start]);
            body.push_str(text);
            from = r.end;
        }
        body.push_str(&old[from..]);
        Ok(body)
    }
}
