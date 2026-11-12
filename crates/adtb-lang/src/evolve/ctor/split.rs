use super::*;
use crate::schema::TypeDef;

/// A split's image of the split constructor `ctor`, named `source`, into `products`.
struct SplitImage<'a> {
    def: TypeId,
    ctor: u32,
    source: &'a str,
    products: &'a [String],
    carries: &'a Carries,
    /// Each product with the names of its fields.
    shapes: &'a [(String, Vec<String>)],
}

/// Returns `cl` restated over each product that keeps every one of its fields.
fn split_images(image: &SplitImage, cl: &ScopedClause, td: &TypeDef) -> Vec<String> {
    let mut images = Vec::new();
    for p in image.products {
        let mut fs = Vec::new();
        for f in &cl.fields {
            let (head, rest) = element_head(f);
            if td.field(None, head).is_some() {
                fs.push(f.clone());
            } else if let Some((_, _, _, g)) = image
                .carries
                .iter()
                .find(|(x, y, z, _)| x == image.source && y == head && z == p)
            {
                fs.push(format!("{g}{rest}"));
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
        self.replacing_carried(&path.ty, &carries);
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
        let shapes: Vec<_> = into.iter().map(carry::ctor_fields).collect();
        let image = SplitImage {
            def,
            ctor: c,
            source: &source,
            products: &products,
            carries: &carries,
            shapes: &shapes,
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
        let td = schema.ty(def);
        let readable: Vec<String> = td
            .header()
            .iter()
            .chain(&td.ctors()[c as usize].fields)
            .map(|f| f.name.clone())
            .collect();
        let routed = carry::routed(&images, std::slice::from_ref(&source), &shapes);
        self.plan_carry(
            carry::Structural::Split,
            &schema,
            (def, &path.ty),
            (std::slice::from_ref(&source), &products),
            (&routed.0, &routed.1, &[], &matched_fields(arms, &readable)),
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
            let td = schema.ty(def);
            let source = image.source;
            let lost = cl.fields.iter().rev().find(|f| {
                let head = element_head(f).0;
                td.field(None, head).is_none()
                    && !image
                        .carries
                        .iter()
                        .any(|(x, y, _, _)| x == source && y == head)
            });
            if scope.len() > 1 {
                let unkept = cl.fields.iter().find_map(|f| {
                    let head = element_head(f).0;
                    if td.field(None, head).is_some() {
                        return None;
                    }
                    let keeps = |(p, fs): &&(String, Vec<String>)| {
                        fs.iter().any(|g| g == head)
                            && !image
                                .carries
                                .iter()
                                .any(|(x, y, q, z)| x == source && y == head && q == p && z != head)
                    };
                    image.shapes.iter().find(|s| !keeps(s)).map(|(p, _)| (f, p))
                });
                if let Some((f, p)) = unkept {
                    if replacements.is_empty() {
                        return self.err(format!(
                            "`{shown} in {}` names `{f}`, which `{p}` does not keep under that name",
                            sel_text(&cl.scope_names())
                        ));
                    }
                    clause_edits.entry(cl.decl).or_default().push((
                        clause_removal(&text, std::slice::from_ref(&cl.range), 0),
                        String::new().into(),
                    ));
                    continue;
                }
                let products = &image.products;
                if let Some(names) = cl.rescoped(&[c], &|_| products.to_vec()) {
                    clause_edits.entry(cl.decl).or_default().push((
                        cl.range.clone(),
                        format!("{shown} in {}", sel_text(&names)).into(),
                    ));
                }
                continue;
            }
            if let Some(f) = lost
                && replacements.is_empty()
            {
                return self.err(format!(
                    "`{shown} in {source}` names `{f}`, which no new constructor keeps; state its replacements after the routing"
                ));
            }
            let kept = |f: &String| {
                let head = element_head(f).0;
                td.field(None, head).is_some()
                    || image.products.iter().all(|p| {
                        image
                            .carries
                            .iter()
                            .any(|(x, y, q, z)| x == source && y == head && q == p && z == head)
                    })
            };
            if cl.rescoped(&[c], &|_| Vec::new()).is_none() && cl.fields.iter().all(kept) {
                continue;
            }
            clause_edits.entry(cl.decl).or_default().push((
                clause_removal(&text, std::slice::from_ref(&cl.range), 0),
                String::new().into(),
            ));
            let list = added.entry(cl.decl).or_default();
            for img in split_images(image, &cl, td) {
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
        if let Ty::Data { def: s, .. } = &inner {
            let st = after.ty(*s);
            let into = (tm.as_str(), td, n.as_str(), field.name.text.as_str());
            self.replacing_nested(into, &after.module(st.module).name, &st.name, &sources);
        }
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
            let rescoped = cl.rescoped(&src_ix, &|_| vec![n.clone()]);
            let mut fs = Vec::new();
            for g in &cl.fields {
                let head = element_head(g).0;
                if td.field(None, head).is_some() {
                    fs.push(g.clone());
                } else if let [one] = inside.as_slice() {
                    let src = &td.ctors()[*one as usize];
                    if src.fields.iter().any(|x| x.name == head) && scope.len() == 1 {
                        fs.push(format!("{f}.{}.{g}", src.name));
                    } else {
                        return self.err(format!("`{shown}` has no image under the nest"));
                    }
                } else {
                    return self.err(format!("`{shown}` has no image under the nest"));
                }
            }
            let names = match rescoped {
                None if fs == cl.fields => continue,
                None => cl.scope_names(),
                Some(names) => {
                    if scope.len() != inside.len() && inside.len() != src_ix.len() {
                        return self.err(format!("`{shown}` ranges over some but not all of the nested constructors and has no image under the nest"));
                    }
                    names
                }
            };
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
            carry::Structural::Nest,
            &schema,
            (def, ty),
            (&sources, std::slice::from_ref(&n)),
            (&reaches, &given, &[], &[]),
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
        let tm = self.module_of(&path.ty);
        self.replacing_unnested((tm.as_str(), td, n.as_str(), field.name.as_str()), &sm, st);
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
                if td.field(None, element_head(g).0).is_some() {
                    fs.push(g.clone());
                    continue;
                }
                let parts: Vec<&str> = g.split('.').map(str::trim).collect();
                match parts.as_slice() {
                    [f, ctor, leaf @ ..]
                        if *f == field.name
                            && !leaf.is_empty()
                            && products.iter().any(|p| p == ctor) =>
                    {
                        if target.as_ref().is_some_and(|t| t != ctor) {
                            return self.err(format!("`{shown}` has no image under the unnest"));
                        }
                        target = Some(ctor.to_string());
                        fs.push(leaf.join("."));
                    }
                    _ => return self.err(format!("`{shown}` has no image under the unnest")),
                }
            }
            let names: Vec<String> = match &target {
                Some(p) if scope.len() == 1 || cl.scope.is_none() => vec![p.clone()],
                Some(_) => return self.err(format!("`{shown}` has no image under the unnest")),
                None if cl.scope.is_none() => continue,
                None => match cl.rescoped(&[c], &|_| products.clone()) {
                    None => continue,
                    Some(names) => names,
                },
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
            carry::Structural::Unnest,
            &schema,
            (def, &path.ty),
            (std::slice::from_ref(&n), &products),
            (&reaches, &given, &[], std::slice::from_ref(&field.name)),
        );
        self.apply_edits(edits);
        Ok(())
    }
}
