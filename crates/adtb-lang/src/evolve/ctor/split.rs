use super::*;

/// A split's image of the split constructor `ctor`, named `source`, into `products`.
struct SplitImage<'a> {
    def: TypeId,
    ctor: u32,
    source: &'a str,
    products: &'a [String],
    carries: &'a Carries,
}

/// Returns `cl` restated over each product that keeps every one of its fields.
fn split_images(image: &SplitImage, cl: &ScopedClause, header: &BTreeSet<&str>) -> Vec<String> {
    let mut images = Vec::new();
    for p in image.products {
        let mut fs = Vec::new();
        for f in &cl.fields {
            if header.contains(f.as_str()) {
                fs.push(f.clone());
            } else if let Some((_, _, _, g)) = image
                .carries
                .iter()
                .find(|(x, y, z, _)| x == image.source && y == f && z == p)
            {
                fs.push(g.clone());
            } else {
                break;
            }
        }
        if fs.len() == cl.fields.len() {
            images.push(format!("{} ({}) in {p}", cl.kind, fs.join(", ")));
        }
    }
    images
}

impl Cx<'_> {
    pub(in crate::evolve) fn split(
        &mut self,
        path: &ast::CtorPath,
        into: &[ast::CtorDef],
        arms: &[ast::Arm],
        clauses: &[ast::SplitClause],
    ) -> R<()> {
        let schema = self.schema()?;
        let (i, t, def, c) = self.ctor_of(&schema, path)?;
        for d in into {
            self.fresh_ctor(&schema, def, d)?;
        }
        let source = path.ctor.text.clone();
        let products: Vec<String> = into.iter().map(|d| d.name.text.clone()).collect();
        let texts: Vec<Written> = into.iter().map(|d| self.ctor_def_text(d)).collect();
        let carries = self.routing(
            &path.ty,
            std::slice::from_ref(&source),
            &products,
            &texts,
            arms,
        )?;
        let refs = check::schema_refs(&schema);
        let mut replacements = Vec::new();
        let mut arm_clauses = Vec::new();
        for cl in clauses {
            match cl {
                ast::SplitClause::Unique(fs, s) => {
                    replacements.push(self.replacement_text("unique", fs, s));
                }
                ast::SplitClause::Index {
                    interval,
                    fields,
                    scope,
                } => replacements.push(self.replacement_text(
                    if *interval { "interval index" } else { "index" },
                    fields,
                    scope,
                )),
                ast::SplitClause::Arm(a) => arm_clauses.push((**a).clone()),
            }
        }
        let mut edits = self.reselect(
            &schema,
            def,
            std::slice::from_ref(&source),
            &products,
            Reselect {
                whole: false,
                clauses: false,
                checks: true,
                what: "the split",
            },
        )?;
        edits = Self::merged(
            edits,
            self.reselect_rows(
                &schema,
                def,
                std::slice::from_ref(&source),
                &products,
                false,
            ),
        );
        let image = SplitImage {
            def,
            ctor: c,
            source: &source,
            products: &products,
            carries: &carries,
        };
        let clause_edits = self.split_clauses(&schema, &image, &replacements)?;
        edits = Self::merged(edits, clause_edits);
        edits = Self::merged(
            edits,
            self.replace_ctors(
                i,
                &t,
                std::slice::from_ref(&source),
                &Written::joined(texts, "\n  | "),
            ),
        );
        let arm_edits = self.arm_replacements(&schema, &refs, def, &[c], &arm_clauses)?;
        edits = Self::merged(edits, arm_edits);
        let tm = self.module_of(&path.ty);
        self.restate(
            &schema,
            def,
            &tm,
            std::slice::from_ref(&source),
            &products,
            false,
        )?;
        let images: Vec<_> = arms.iter().map(arm_image).collect();
        let shapes: Vec<_> = into.iter().map(carry::ctor_fields).collect();
        let routed = carry::routed(&images, std::slice::from_ref(&source), &shapes);
        self.plan_carry(
            "split",
            &schema,
            (def, &path.ty),
            (std::slice::from_ref(&source), &products),
            (&routed.0, &routed.1, &[]),
        );
        self.apply_edits(edits);
        self.retype_products(&schema, def, std::slice::from_ref(&source), &products)
    }

    /// Returns the edits that rescope each clause over the split constructor to its products,
    /// and add `replacements` to every relation over the type.
    fn split_clauses(
        &self,
        schema: &Schema,
        image: &SplitImage,
        replacements: &[Written],
    ) -> R<Edits> {
        let (def, c) = (image.def, image.ctor);
        let mut clause_edits: Edits = BTreeMap::new();
        let mut added: BTreeMap<usize, Vec<Written>> = BTreeMap::new();
        let mut decls_of_t = BTreeSet::new();
        for cl in self.scoped_clauses(schema, def)? {
            decls_of_t.insert(cl.decl);
            let Some(scope) = &cl.scope else { continue };
            if !scope.contains(&c) {
                continue;
            }
            let shown = format!("{} ({})", cl.kind, cl.fields.join(", "));
            let text = self.src.decls[cl.decl].text.clone();
            if scope.len() > 1 {
                let td = schema.ty(def);
                let mut names = Vec::new();
                for &x in scope {
                    if x == c {
                        names.extend(image.products.iter().cloned());
                    } else {
                        names.push(td.ctors()[x as usize].name.clone());
                    }
                }
                clause_edits.entry(cl.decl).or_default().push((
                    cl.range.clone(),
                    format!("{shown} in {}", sel_text(&names)).into(),
                ));
                continue;
            }
            let header: BTreeSet<&str> = schema
                .ty(def)
                .header()
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            let source = image.source;
            let lost = cl.fields.iter().rev().find(|f| {
                !header.contains(f.as_str())
                    && !image
                        .carries
                        .iter()
                        .any(|(x, y, _, _)| x == source && y == *f)
            });
            if let Some(f) = lost
                && replacements.is_empty()
            {
                return self.err(format!(
                    "`{shown} in {source}` names `{f}`, which no new constructor keeps; state its replacements after the routing"
                ));
            }
            clause_edits.entry(cl.decl).or_default().push((
                clause_removal(&text, std::slice::from_ref(&cl.range), 0),
                String::new().into(),
            ));
            let list = added.entry(cl.decl).or_default();
            for img in split_images(image, &cl, &header) {
                if !list.iter().any(|x| x.text == img) {
                    list.push(img.into());
                }
            }
        }
        for k in decls_of_t {
            let list = added.entry(k).or_default();
            for r in replacements {
                match list.iter_mut().find(|x| x.text == r.text) {
                    Some(x) => *x = r.clone(),
                    None => list.push(r.clone()),
                }
            }
        }
        for (k, news) in added {
            if news.is_empty() {
                continue;
            }
            let end = self.src.decls[k].text.trim_end().len();
            clause_edits
                .entry(k)
                .or_default()
                .push((end..end, Written::prefixed(news, "\n  ")));
        }
        Ok(clause_edits)
    }

    pub(in crate::evolve) fn nest(
        &mut self,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        clauses: &[ast::ArmClause],
    ) -> R<()> {
        let schema = self.schema()?;
        let (i, t) = self.type_decl(ty)?;
        let def = self.type_id(&schema, ty)?;
        let sources = self.sources(&schema, def, sel)?;
        self.fresh_ctor(&schema, def, into)?;
        let n = into.name.text.clone();
        let fields = into.record.as_deref().unwrap_or_default();
        let [field] = fields else {
            return self.err_at(
                into.name.span,
                format!("the nesting constructor `{n}` has exactly one field"),
            );
        };
        let n_text = self.ctor_def_text(into);
        let after = self.with_ctors(i, &t, std::slice::from_ref(&n_text))?;
        let ad = self.type_id(&after, ty)?;
        let nc = after
            .ty(ad)
            .ctor_index(&n)
            .expect("the nesting constructor");
        let inner = after.ty(ad).ctors()[nc as usize].fields[0].ty.clone();
        let td = schema.ty(def);
        let same = match &inner {
            Ty::Data {
                def: s,
                args,
                ctors: None,
            } if args.is_empty() => {
                let st = after.ty(*s);
                st.header().is_empty()
                    && st.ctors().len() == sources.len()
                    && sources.iter().all(|src| {
                        let k = td.ctor_index(src).expect("a source");
                        let a = &td.ctors()[k as usize].fields;
                        st.ctor_index(src).is_some_and(|j| {
                            let b = &st.ctors()[j as usize].fields;
                            a.len() == b.len()
                                && a.iter()
                                    .all(|f| b.iter().any(|g| g.name == f.name && g.ty == f.ty))
                        })
                    })
            }
            _ => false,
        };
        if !same {
            return self.err_at(field.ty.span, format!(
                "the field of `{n}` holds an existing sum whose constructors have the names and fields of {}",
                sources.join(", ")
            ));
        }
        let src_ix: Vec<u32> = sources.iter().filter_map(|s| td.ctor_index(s)).collect();
        let refs = check::schema_refs(&schema);
        let tm = self.module_of(ty);
        self.restate(&schema, def, &tm, &sources, std::slice::from_ref(&n), true)?;
        let mut edits = self.reselect(
            &schema,
            def,
            &sources,
            std::slice::from_ref(&n),
            Reselect {
                whole: true,
                clauses: false,
                checks: true,
                what: "the nest",
            },
        )?;
        let mut clause_edits: Edits = BTreeMap::new();
        let header: BTreeSet<&str> = td.header().iter().map(|f| f.name.as_str()).collect();
        let f = field.name.text.clone();
        for cl in self.scoped_clauses(&schema, def)? {
            let Some(scope) = &cl.scope else { continue };
            let inside: Vec<u32> = scope
                .iter()
                .copied()
                .filter(|c| src_ix.contains(c))
                .collect();
            if inside.is_empty() {
                continue;
            }
            let shown = format!("{} ({})", cl.kind, cl.fields.join(", "));
            let mut names: Vec<String> = Vec::new();
            for &x in scope {
                let nm = if src_ix.contains(&x) {
                    n.clone()
                } else {
                    td.ctors()[x as usize].name.clone()
                };
                if !names.contains(&nm) {
                    names.push(nm);
                }
            }
            let mut fs = Vec::new();
            for g in &cl.fields {
                if header.contains(g.as_str()) {
                    fs.push(g.clone());
                } else if let [one] = inside.as_slice() {
                    let src = &td.ctors()[*one as usize];
                    if src.fields.iter().any(|x| x.name == *g) && scope.len() == 1 {
                        fs.push(format!("{f}.{}.{g}", src.name));
                    } else {
                        return self.err(format!("`{shown}` has no image under the nest"));
                    }
                } else {
                    return self.err(format!("`{shown}` has no image under the nest"));
                }
            }
            if scope.len() != inside.len() && inside.len() != src_ix.len() {
                return self.err(format!("`{shown}` ranges over some but not all of the nested constructors and has no image under the nest"));
            }
            clause_edits.entry(cl.decl).or_default().push((
                cl.range.clone(),
                format!("{} ({}) in {}", cl.kind, fs.join(", "), sel_text(&names)).into(),
            ));
        }
        edits = Self::merged(edits, clause_edits);
        edits = Self::merged(edits, self.replace_ctors(i, &t, &sources, &n_text));
        let arm_edits = self.arm_replacements(&schema, &refs, def, &src_ix, clauses)?;
        edits = Self::merged(edits, arm_edits);
        let mut given = carry::Given::new();
        for s in &sources {
            let k = td.ctor_index(s).expect("a source");
            given.insert(
                (s.clone(), n.clone(), f.clone()),
                td.ctors()[k as usize]
                    .fields
                    .iter()
                    .map(|x| Some(x.name.clone()))
                    .collect(),
            );
        }
        let reaches = sources.iter().map(|s| (s.clone(), n.clone())).collect();
        self.plan_carry(
            "nest",
            &schema,
            (def, ty),
            (&sources, std::slice::from_ref(&n)),
            (&reaches, &given, &[]),
        );
        self.apply_edits(edits);
        Ok(())
    }

    pub(in crate::evolve) fn unnest(
        &mut self,
        path: &ast::CtorPath,
        clauses: &[ast::ArmClause],
    ) -> R<()> {
        let schema = self.schema()?;
        let (i, t, def, c) = self.ctor_of(&schema, path)?;
        let td = schema.ty(def);
        let n = path.ctor.text.clone();
        let fields = &td.ctors()[c as usize].fields;
        let [field] = fields.as_slice() else {
            return self.err_at(
                path.ctor.span,
                format!("`{n}` holds one field of a sum, which unnest lifts"),
            );
        };
        let Ty::Data { def: s, args, .. } = &field.ty else {
            return self.err_at(
                path.ctor.span,
                format!("the field of `{n}` holds no declared sum"),
            );
        };
        let st = schema.ty(*s);
        if !args.is_empty()
            || !st.header().is_empty()
            || !matches!(st.kind, TypeDefKind::Sum { record: false, .. })
        {
            return self.err_at(
                path.ctor.span,
                format!("the field of `{n}` holds no declared sum without a header and parameters"),
            );
        }
        let products: Vec<String> = st.ctors().iter().map(|x| x.name.clone()).collect();
        for p in &products {
            if *p != n && td.ctor_index(p).is_some() {
                return self.err(format!("`{p}` is already a constructor of `{}`", td.name));
            }
        }
        let sm = schema.module(st.module).name.clone();
        let Some(si) = self.src.find("type", &sm, &st.name) else {
            return self.err(format!("`{}` is not a type of the schema", st.name));
        };
        let Some(Decl::Type(sd)) = self.src.parsed(si) else {
            return self.err(format!("`{}` is not a type of the schema", st.name));
        };
        let sh = header(&sm).len();
        let stext = self.src.decls[si].text.clone();
        let texts: Vec<String> = ctor_extents(&stext, sh, &sd)
            .into_iter()
            .map(|(_, name, end)| stext[name.start..end].to_string())
            .collect();
        let refs = check::schema_refs(&schema);
        let mut edits = self.reselect(
            &schema,
            def,
            std::slice::from_ref(&n),
            &products,
            Reselect {
                whole: false,
                clauses: false,
                checks: true,
                what: "the unnest",
            },
        )?;
        let mut clause_edits: Edits = BTreeMap::new();
        let header: BTreeSet<&str> = td.header().iter().map(|f| f.name.as_str()).collect();
        let every: Vec<u32> = (0..td.ctors().len() as u32).collect();
        for cl in self.scoped_clauses(&schema, def)? {
            let scope = cl.scope.as_ref().unwrap_or(&every);
            if !scope.contains(&c) {
                continue;
            }
            let shown = format!("{} ({})", cl.kind, cl.fields.join(", "));
            let mut target: Option<String> = None;
            let mut fs = Vec::new();
            for g in &cl.fields {
                if header.contains(g.as_str()) {
                    fs.push(g.clone());
                    continue;
                }
                let parts: Vec<&str> = g.split('.').map(str::trim).collect();
                match parts.as_slice() {
                    [f, ctor, leaf] if *f == field.name && products.iter().any(|p| p == ctor) => {
                        if target.as_ref().is_some_and(|t| t != ctor) {
                            return self.err(format!("`{shown}` has no image under the unnest"));
                        }
                        target = Some(ctor.to_string());
                        fs.push(leaf.to_string());
                    }
                    _ => return self.err(format!("`{shown}` has no image under the unnest")),
                }
            }
            let td_names = |x: u32| td.ctors()[x as usize].name.clone();
            let names: Vec<String> = match &target {
                Some(p) if scope.len() == 1 || cl.scope.is_none() => vec![p.clone()],
                Some(_) => return self.err(format!("`{shown}` has no image under the unnest")),
                None if cl.scope.is_none() => continue,
                None => scope
                    .iter()
                    .flat_map(|&x| {
                        if x == c {
                            products.clone()
                        } else {
                            vec![td_names(x)]
                        }
                    })
                    .collect(),
            };
            clause_edits.entry(cl.decl).or_default().push((
                cl.range.clone(),
                format!("{} ({}) in {}", cl.kind, fs.join(", "), sel_text(&names)).into(),
            ));
        }
        edits = Self::merged(edits, clause_edits);
        edits = Self::merged(
            edits,
            self.replace_ctors(
                i,
                &t,
                std::slice::from_ref(&n),
                &texts.join("\n  | ").into(),
            ),
        );
        let arm_edits = self.arm_replacements(&schema, &refs, def, &[c], clauses)?;
        edits = Self::merged(edits, arm_edits);
        let tm = self.module_of(&path.ty);
        self.restate(
            &schema,
            def,
            &tm,
            std::slice::from_ref(&n),
            &products,
            false,
        )?;
        let mut given = carry::Given::new();
        for x in st.ctors() {
            for g in &x.fields {
                given.insert(
                    (n.clone(), x.name.clone(), g.name.clone()),
                    vec![Some(field.name.clone())],
                );
            }
        }
        let reaches = products.iter().map(|p| (n.clone(), p.clone())).collect();
        self.plan_carry(
            "unnest",
            &schema,
            (def, &path.ty),
            (std::slice::from_ref(&n), &products),
            (&reaches, &given, &[]),
        );
        self.apply_edits(edits);
        Ok(())
    }
}
