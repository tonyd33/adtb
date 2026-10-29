use super::*;

mod merge;
mod remove;
mod split;

impl Cx<'_> {
    /// The text of an `arm` clause's arm: its pattern through its body.
    pub(super) fn arm_text(&self, a: &ast::ArmClause) -> Written {
        self.copy_range(a.pattern.span.start as usize..a.body.span.end as usize)
    }

    /// The declaration an `arm` clause names, and for a computed field the span of its expression.
    pub(super) fn arm_target(&self, a: &ast::ArmClause) -> R<(usize, Option<Span>)> {
        let m = self.module_of(&a.target);
        if let Some(f) = &a.field {
            let (i, r) = self.relation_decl(&a.target)?;
            for c in &r.clauses {
                if let RelationClause::Computed(n, _, e) = c
                    && n.text == f.text
                {
                    return Ok((i, Some(e.span)));
                }
            }
            return self.err_at(
                f.span,
                format!(
                    "`{}` has no computed field `{}`",
                    a.target.name.text, f.text
                ),
            );
        }
        for kind in ["fn", "constraint"] {
            if let Some(i) = self.src.find(kind, &m, &a.target.name.text) {
                return Ok((i, None));
            }
        }
        self.err_at(
            a.target.span(),
            format!(
                "module `{m}` declares no schema function, constraint or computed field `{}`",
                a.target.name.text
            ),
        )
    }

    /// The one match of declaration `i` (within `within`) over a value that covers constructor `c` of `def`.
    pub(super) fn target_match(
        &self,
        refs: &[(FileId, Span, Ref)],
        a: &ast::ArmClause,
        i: usize,
        within: Option<Span>,
        covers: &dyn Fn(&Ty) -> bool,
    ) -> R<Span> {
        let mut found: Vec<Span> = refs
            .iter()
            .filter(|(f, s, r)| {
                f.0 as usize == i
                    && within.is_none_or(|w| w.start <= s.start && s.end <= w.end)
                    && matches!(r, Ref::Match(t) if covers(t))
            })
            .map(|(_, s, _)| *s)
            .collect();
        found.sort_by_key(|s| (s.start, s.end));
        found.dedup();
        match found.as_slice() {
            [one] => Ok(*one),
            [] => self.err_at(
                a.target.span(),
                format!(
                    "`{}` has no match over the constructors this arm is for; redefine it in the same migration instead",
                    a.target.name.text
                ),
            ),
            _ => self.err_at(
                a.target.span(),
                format!(
                    "`{}` has more than one match over the constructors this arm is for; redefine it in the same migration instead",
                    a.target.name.text
                ),
            ),
        }
    }

    /// The edits of declaration `i` that replace, in the match at `at`, the arms naming a constructor in `replaced` of `def` by `arms`.
    pub(super) fn replace_arms(
        &self,
        refs: &[(FileId, Span, Ref)],
        i: usize,
        at: Span,
        def: TypeId,
        replaced: &[u32],
        arms: &[Written],
    ) -> Vec<(Range<usize>, Written)> {
        let h = header(&self.src.decls[i].module).len();
        let text = &self.src.decls[i].text;
        let Some(d) = self.src.parsed(i) else {
            return Vec::new();
        };
        let Some(old) = match_arms(&d, at) else {
            return Vec::new();
        };
        let mut edits = Vec::new();
        for a in &old {
            let names_replaced = a.patterns.iter().any(|p| {
                let ast::PatternKind::Ctor { path, ctor, .. } = &p.kind else {
                    return false;
                };
                let name = ctor.as_ref().unwrap_or(&path.name).span;
                refs.iter().any(|(f, s, r)| {
                    f.0 as usize == i
                        && *s == name
                        && matches!(r, Ref::Ctor(d2, c) if *d2 == def && replaced.contains(c))
                })
            });
            if names_replaced {
                edits.push((arm_range(text, h, a), Written::default()));
            }
        }
        let end = match_end(&d, at).unwrap_or(at.end as usize) - h;
        edits.push((end..end, Written::prefixed(arms.iter().cloned(), "\n  | ")));
        edits
    }

    /// Rewrite record `t`, declaration `i`, as a sum of one constructor named after it, the key
    /// fields of its relations moving to the header.
    fn record_to_sum(
        &mut self,
        i: usize,
        t: &ast::TypeDecl,
        path: &ast::CtorPath,
        fields: &[ast::FieldType],
    ) -> R<()> {
        let tname = &t.name.text;
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let Some(braces) = field_braces(&text, h, t, None) else {
            return self.err_at(
                path.ty.span(),
                format!("record `{tname}` has no fields to keep"),
            );
        };
        let schema = self.schema()?;
        let keys = Self::key_fields(&schema, self.type_id(&schema, &path.ty)?);
        let item = |f: &&ast::FieldType| {
            let end = f.on_delete.as_ref().map_or(f.ty.span.end, |a| a.span.end);
            text[f.name.span.start as usize - h..end as usize - h].to_string()
        };
        let (head, rest): (Vec<&ast::FieldType>, Vec<&ast::FieldType>) =
            fields.iter().partition(|f| keys.contains(&f.name.text));
        let edit = if head.is_empty() {
            (braces.start..braces.start, format!("{tname} "))
        } else {
            let eq = text[..braces.start].rfind('=').expect("a record's `=`");
            let body = if rest.is_empty() {
                tname.clone()
            } else {
                let rest: Vec<String> = rest.iter().map(item).collect();
                format!("{tname} {{ {} }}", rest.join(", "))
            };
            let head: Vec<String> = head.iter().map(item).collect();
            (
                eq..braces.end,
                format!("{{ {} }}\n  = {body}", head.join(", ")),
            )
        };
        self.src.edit(i, vec![edit]);
        Ok(())
    }

    /// Returns the key fields of the relations over type `def`.
    fn key_fields(schema: &Schema, def: TypeId) -> BTreeSet<String> {
        Self::relations_of(schema, def)
            .into_iter()
            .flat_map(|(r, _, _)| schema.relation(r).key.clone())
            .collect()
    }

    pub(super) fn add_constructor(
        &mut self,
        path: &ast::CtorPath,
        has_record: bool,
        place: Option<&CtorPlace>,
        joins: &[ast::RelationRef],
        arms: &[ast::ArmClause],
    ) -> R<()> {
        let (i, mut t) = self.type_decl(&path.ty)?;
        let name = &path.ctor.text;
        let tname = &t.name.text.clone();
        match &t.body {
            ast::TypeBody::Ctors(_) => {}
            ast::TypeBody::Record(_) if name == tname => {
                return self.err_at(
                    path.ctor.span,
                    format!("`{name}` is already a constructor of `{tname}`"),
                );
            }
            ast::TypeBody::Record(fields) => {
                self.record_to_sum(i, &t, path, fields)?;
                t = self.type_decl(&path.ty)?.1;
            }
            ast::TypeBody::New(..) => {
                return self.err_at(
                    path.ty.span(),
                    format!("newtype `{tname}` has no constructors"),
                );
            }
        }
        let before = self.schema()?;
        let def = self.type_id(&before, &path.ty)?;
        if before.ty(def).ctor_index(name).is_some() {
            return self.err_at(
                path.ctor.span,
                format!("`{name}` is already a constructor of `{tname}`"),
            );
        }
        let rels = Self::relations_of(&before, def);
        for (rel, _, rname) in &rels {
            if before.relation(*rel).sets.iter().any(|s| s.name == *name) {
                return self.err_at(
                    path.ctor.span,
                    format!("`{name}` is already a named set of `{rname}`"),
                );
            }
        }
        let ctor_text = self.copy_range(ctor_extent(self.text, &path.ctor, has_record));
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let ctors = ctor_extents(&text, h, &t);
        let edit = match place {
            None => {
                let at = ctors.last().map_or(text.trim_end().len(), |c| c.2);
                (at..at, Written::prefixed([ctor_text], "\n  | "))
            }
            Some(CtorPlace::Before(c) | CtorPlace::After(c)) => {
                let Some(k) = ctors.iter().position(|x| x.0 == c.text) else {
                    return self.err_at(
                        c.span,
                        format!("`{}` is not a constructor of `{tname}`", c.text),
                    );
                };
                match place {
                    Some(CtorPlace::Before(_)) => {
                        let at = ctors[k].1.start;
                        (at..at, Written::concat([ctor_text, "\n  | ".into()]))
                    }
                    _ => {
                        let at = ctors[k].2;
                        (at..at, Written::prefixed([ctor_text], "\n  | "))
                    }
                }
            }
        };
        self.src.edit(i, vec![edit]);
        for j in joins {
            let (_, rd) = self.relation_decl(&j.relation)?;
            let rel = self.rel_id(&before, &j.relation)?;
            if before.relation(rel).elem_def != def {
                return self.err_at(
                    j.span,
                    format!(
                        "`{}` is not a relation variable of `{tname}`",
                        rd.name.name.text
                    ),
                );
            }
            let Some(CtorSel::One(set)) = &j.sel else {
                return self.err_at(j.span, "`joins` names a named set: `r.S`");
            };
            let (k, sd) = self.set_decl(set, &j.relation)?;
            let at = sd.span.end as usize - header(&self.src.decls[k].module).len();
            self.src.edit(k, vec![(at..at, format!(" | {name}"))]);
        }
        let since = self.src.version;
        for (_, m, r) in &rels {
            self.src.states.insert(
                (m.clone(), r.clone(), name.clone()),
                State {
                    stage: Stage::Declared,
                    since,
                },
            );
        }
        self.src.value_states.insert(
            (self.module_of(&path.ty), tname.clone(), name.clone()),
            State {
                stage: Stage::Declared,
                since,
            },
        );
        if arms.is_empty() {
            return Ok(());
        }
        let after = self.schema()?;
        let c = after
            .ty(def)
            .ctor_index(name)
            .expect("the added constructor");
        let refs = check::schema_refs(&after);
        let mut inserts: BTreeMap<(usize, usize), Written> = BTreeMap::new();
        for a in arms {
            let (k, within) = self.arm_target(a)?;
            let at = self.target_match(&refs, a, k, within, &|ty| holds(&after, ty, def, c))?;
            let end = self
                .src
                .parsed(k)
                .and_then(|d| match_end(&d, at))
                .unwrap_or(at.end as usize)
                - header(&self.src.decls[k].module).len();
            inserts
                .entry((k, end))
                .or_default()
                .append(Written::prefixed([self.arm_text(a)], "\n  | "));
        }
        self.insert_all(inserts);
        Ok(())
    }
}

impl Cx<'_> {
    /// Applies insertions keyed by declaration and position.
    pub(super) fn insert_all(&mut self, inserts: BTreeMap<(usize, usize), Written>) {
        let mut by_decl: Edits = BTreeMap::new();
        for ((k, at), s) in inserts {
            by_decl.entry(k).or_default().push((at..at, s));
        }
        for (k, es) in by_decl {
            self.src.edit(k, es);
        }
    }

    pub(super) fn apply_edits(&mut self, edits: Edits) {
        for (k, es) in edits {
            self.src.edit(k, es);
        }
    }

    /// The constructor `path` names: its type's declaration and id, and its index.
    pub(super) fn ctor_of(
        &self,
        schema: &Schema,
        path: &ast::CtorPath,
    ) -> R<(usize, ast::TypeDecl, TypeId, u32)> {
        let (i, t) = self.type_decl(&path.ty)?;
        let def = self.type_id(schema, &path.ty)?;
        let Some(c) = schema.ty(def).ctor_index(&path.ctor.text) else {
            return self.err_at(
                path.ctor.span,
                format!(
                    "`{}` is not a constructor of `{}`",
                    path.ctor.text, t.name.text
                ),
            );
        };
        Ok((i, t, def, c))
    }

    /// The edits that remove, from every set, scope and reference type over `def`, the constructors `gone`,
    /// putting `by` in their place when every one of them was there; and the refusals for a selection that
    /// holds only some of them, when `whole` asks for all or none.
    pub(super) fn reselect(
        &self,
        schema: &Schema,
        def: TypeId,
        gone: &[String],
        by: &[String],
        mode: Reselect,
    ) -> R<Edits> {
        let Reselect {
            whole,
            clauses,
            checks,
            what,
        } = mode;
        let mut edits: Edits = BTreeMap::new();
        let mut errs = Vec::new();
        let rewrite = |names: &[String]| reselected(names, gone, by, whole);
        for (rel, m, rname) in Self::relations_of(schema, def) {
            for j in 0..self.src.decls.len() {
                if self.src.decls[j].module != m {
                    continue;
                }
                let Some(Decl::Set(sd)) = self.src.parsed(j) else {
                    continue;
                };
                if sd.relation.text != rname {
                    continue;
                }
                let names: Vec<String> = sd.members.iter().map(|x| x.text.clone()).collect();
                if let Some(out) = rewrite(&names) {
                    let Some(out) = out else {
                        errs.push(Diag::new(
                            self.span,
                            format!(
                                "{what} is refused: the named set `{}` of `{rname}` holds some but not all of its constructors",
                                sd.name.text
                            ),
                        ));
                        continue;
                    };
                    if out.is_empty() {
                        continue;
                    }
                    let h = header(&m).len();
                    let from = sd.members[0].span.start as usize - h;
                    let to = sd.members.last().expect("a member").span.end as usize - h;
                    edits
                        .entry(j)
                        .or_default()
                        .push((from..to, out.join(" | ").into()));
                }
            }
            let sets: BTreeSet<String> = schema
                .relation(rel)
                .sets
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let mut found: Vec<(usize, Vec<ast::Ident>)> = Vec::new();
            for j in 0..self.src.decls.len() {
                let Some(d) = self.src.parsed(j) else {
                    continue;
                };
                let home = self.src.decls[j].module.clone();
                let names_rel = |qn: &QName| {
                    qn.name.text == rname && qn.module.as_ref().map_or(home == m, |x| x.text == m)
                };
                let mut sels: Vec<Vec<ast::Ident>> = Vec::new();
                each_type(&d, &mut |t| {
                    if let ast::TypeKind::Key(_, rr) = &t.kind
                        && names_rel(&rr.relation)
                        && let Some(sel) = &rr.sel
                    {
                        sels.push(sel_idents(sel).into_iter().cloned().collect());
                    }
                });
                match &d {
                    Decl::Relation(rd) if clauses && home == m && rd.name.name.text == rname => {
                        for c in &rd.clauses {
                            if let RelationClause::Unique(_, Some(s))
                            | RelationClause::Index { scope: Some(s), .. } = c
                            {
                                sels.push(scope_idents(s).into_iter().cloned().collect());
                            }
                        }
                    }
                    Decl::Check(c) if checks && names_rel(&c.on.relation) => {
                        if let Some(sel) = &c.on.sel {
                            sels.push(sel_idents(sel).into_iter().cloned().collect());
                        }
                    }
                    _ => {}
                }
                for s in sels {
                    found.push((j, s));
                }
            }
            for (j, idents) in found {
                if idents.iter().any(|x| sets.contains(&x.text)) {
                    continue;
                }
                let names: Vec<String> = idents.iter().map(|x| x.text.clone()).collect();
                let Some(out) = rewrite(&names) else { continue };
                let Some(out) = out else {
                    errs.push(Diag::new(
                        self.span,
                        format!(
                            "{what} is refused: `{rname}.{}` names some but not all of its constructors",
                            sel_text(&names)
                        ),
                    ));
                    continue;
                };
                if out.is_empty() {
                    continue;
                }
                edits
                    .entry(j)
                    .or_default()
                    .push((self.sel_range(j, &idents), sel_text(&out).into()));
            }
        }
        if errs.is_empty() {
            Ok(edits)
        } else {
            Err(errs)
        }
    }

    /// The range of declaration `j` a selection of `idents` spans, its parentheses included.
    fn sel_range(&self, j: usize, idents: &[ast::Ident]) -> Range<usize> {
        let h = header(&self.src.decls[j].module).len();
        let text = &self.src.decls[j].text;
        let from = idents[0].span.start as usize - h;
        let to = idents.last().expect("an ident").span.end as usize - h;
        if idents.len() > 1 || text[..from].trim_end().ends_with('(') {
            let open = text[..from].trim_end().len() - 1;
            let close = to + text[to..].find(')').map_or(0, |k| k + 1);
            open..close
        } else {
            from..to
        }
    }

    /// The edits that put `by` in place of `gone` in every selection of constructors of `def` an
    /// expression or a row type makes, such as `from l in places.Location` or `(p: Place.Location)`.
    /// With `whole`, a selection of only some of `gone` is left as it is.
    pub(super) fn reselect_rows(
        &self,
        schema: &Schema,
        def: TypeId,
        gone: &[String],
        by: &[String],
        whole: bool,
    ) -> Edits {
        let td = schema.ty(def);
        let gone_ix: Vec<u32> = gone.iter().filter_map(|g| td.ctor_index(g)).collect();
        let noted: BTreeSet<(usize, u32)> = check::schema_refs(schema)
            .into_iter()
            .filter(|(_, _, r)| matches!(r, Ref::Ctor(d, c) if *d == def && gone_ix.contains(c)))
            .map(|(f, s, _)| (f.0 as usize, s.start))
            .collect();
        let type_module = schema.module(td.module).name.clone();
        let mut edits: Edits = BTreeMap::new();
        for j in 0..self.src.decls.len() {
            let Some(d) = self.src.parsed(j) else {
                continue;
            };
            let home = self.src.decls[j].module.clone();
            let mut sels: Vec<Vec<ast::Ident>> = Vec::new();
            for e in decl_exprs(&d) {
                crate::visit::each_expr(e, &mut |x| {
                    if let ast::ExprKind::Select(_, sel) = &x.kind
                        && sel_idents(sel)
                            .iter()
                            .any(|i| noted.contains(&(j, i.span.start)))
                    {
                        sels.push(sel_idents(sel).into_iter().cloned().collect());
                    }
                });
            }
            each_type(&d, &mut |t| {
                if let ast::TypeKind::Named {
                    name,
                    sel: Some(sel),
                    ..
                } = &t.kind
                    && name.name.text == td.name
                    && name
                        .module
                        .as_ref()
                        .map_or(home == type_module, |m| m.text == type_module)
                {
                    sels.push(sel_idents(sel).into_iter().cloned().collect());
                }
            });
            for idents in sels {
                let names: Vec<String> = idents.iter().map(|x| x.text.clone()).collect();
                if let Some(Some(out)) = reselected(&names, gone, by, whole)
                    && !out.is_empty()
                {
                    edits
                        .entry(j)
                        .or_default()
                        .push((self.sel_range(j, &idents), sel_text(&out).into()));
                }
            }
        }
        edits
    }

    pub(super) fn rename_constructor(&mut self, path: &ast::CtorPath, to: &ast::Ident) -> R<()> {
        let schema = self.schema()?;
        let (i, t, def, c) = self.ctor_of(&schema, path)?;
        let from = path.ctor.text.clone();
        if schema.ty(def).ctor_index(&to.text).is_some() {
            return self.err_at(
                to.span,
                format!(
                    "`{}` is already a constructor of `{}`",
                    to.text, t.name.text
                ),
            );
        }
        let rels = Self::relations_of(&schema, def);
        for (rel, _, r) in &rels {
            if schema.relation(*rel).sets.iter().any(|s| s.name == to.text) {
                return self.err_at(
                    to.span,
                    format!("`{}` is already a named set of `{r}`", to.text),
                );
            }
        }
        let mut edits = self.reselect(
            &schema,
            def,
            std::slice::from_ref(&from),
            std::slice::from_ref(&to.text),
            Reselect {
                whole: false,
                clauses: true,
                checks: true,
                what: "the rename",
            },
        )?;
        let h = header(&self.src.decls[i].module).len();
        if let ast::TypeBody::Ctors(cs) = &t.body
            && let Some(cd) = cs.iter().find(|x| x.name.text == from)
        {
            edits
                .entry(i)
                .or_default()
                .push((self.src.local(i, cd.name.span), to.text.clone().into()));
        }
        for (_, f) in decl_fields(&t) {
            if let Some(ast::RefAction::Retag(x, _)) = f.on_delete.as_ref().map(|a| &a.action)
                && x.text == from
            {
                edits.entry(i).or_default().push((
                    x.span.start as usize - h..x.span.end as usize - h,
                    to.text.clone().into(),
                ));
            }
        }
        for (file, span, r) in check::schema_refs(&schema) {
            if r == Ref::Ctor(def, c) {
                let j = file.0 as usize;
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), to.text.clone().into()));
            }
        }
        for (_, m, rname) in &rels {
            let Some(k) = self.src.find("relation", m, rname) else {
                continue;
            };
            let Some(Decl::Relation(rd)) = self.src.parsed(k) else {
                continue;
            };
            for cl in &rd.clauses {
                if let RelationClause::Default { ctor: Some(x), .. } = cl
                    && x.text == from
                {
                    edits
                        .entry(k)
                        .or_default()
                        .push((self.src.local(k, x.span), to.text.clone().into()));
                }
            }
        }
        for j in 0..self.src.decls.len() {
            let Some(d) = self.src.parsed(j) else {
                continue;
            };
            let mut spans = Vec::new();
            each_type(&d, &mut |ty| {
                if let ast::TypeKind::Named {
                    name,
                    sel: Some(sel),
                    ..
                } = &ty.kind
                    && name.name.text == t.name.text
                {
                    for x in sel_idents(sel) {
                        if x.text == from {
                            spans.push(x.span);
                        }
                    }
                }
            });
            for s in spans {
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, s), to.text.clone().into()));
            }
        }
        self.apply_edits(edits);
        let rel_keys: BTreeSet<(String, String)> = rels
            .iter()
            .map(|(_, m, r)| (m.clone(), r.clone()))
            .collect();
        let old: Vec<_> = self.src.states.keys().cloned().collect();
        for k in old {
            if rel_keys.contains(&(k.0.clone(), k.1.clone())) && k.2 == from {
                let v = self.src.states.remove(&k).expect("a key");
                self.src.states.insert((k.0, k.1, to.text.clone()), v);
            }
        }
        let tm = self.module_of(&path.ty);
        if let Some(v) = self
            .src
            .value_states
            .remove(&(tm.clone(), t.name.text.clone(), from))
        {
            self.src
                .value_states
                .insert((tm, t.name.text, to.text.clone()), v);
        }
        Ok(())
    }
}

/// Collects the relation and constructor set of every `Key` type in `t`.
pub(super) fn key_types(t: &Ty, out: &mut Vec<(RelId, Option<CtorSet>)>) {
    match t {
        Ty::Key { rel, ctors } => out.push((*rel, ctors.clone())),
        Ty::Data { args, .. } => args.iter().for_each(|a| key_types(a, out)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => key_types(x, out),
        Ty::Group(a, b) | Ty::Fn(a, b) => {
            key_types(a, out);
            key_types(b, out);
        }
        Ty::Record(fs) => fs.iter().for_each(|(_, x)| key_types(x, out)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .for_each(|(_, fs)| fs.iter().for_each(|(_, x)| key_types(x, out))),
        Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => {}
    }
}

/// A clause of a relation over a type, whose scope a structural operator rewrites.
pub(super) struct ScopedClause {
    pub(super) decl: usize,
    pub(super) range: Range<usize>,
    /// `unique`, `index` or `interval index`.
    pub(super) kind: &'static str,
    pub(super) fields: Vec<String>,
    /// The constructors it ranges over, every one when `None`.
    pub(super) scope: Option<CtorSet>,
}

/// What a fill or a routing checks against.
pub(super) struct RowTypes {
    pub(super) schema: Schema,
    pub(super) program: Program,
    pub(super) module: ModuleId,
    pub(super) row: Ty,
    pub(super) result: Ty,
}

/// What a routing carries whole: (source constructor, field, product constructor, field).
pub(super) type Carries = Vec<(String, String, String, String)>;

impl Cx<'_> {
    pub(super) fn ctor_def_text(&self, d: &ast::CtorDef) -> Written {
        self.copy_range(ctor_extent(self.text, &d.name, d.record.is_some()))
    }

    /// Every unique and index clause of a relation of `def`.
    pub(super) fn scoped_clauses(&self, schema: &Schema, def: TypeId) -> R<Vec<ScopedClause>> {
        let mut out = Vec::new();
        for (rel, m, rname) in Self::relations_of(schema, def) {
            let q = QName {
                module: Some(ast::Ident {
                    text: m.clone(),
                    span: self.span,
                }),
                name: ast::Ident {
                    text: rname.clone(),
                    span: self.span,
                },
            };
            let (i, rd, ranges) = self.clauses(&q)?;
            let h = header(&m).len();
            let text = &self.src.decls[i].text;
            for (k, cl) in rd.clauses.iter().enumerate() {
                let (kind, fs, s) = match cl {
                    RelationClause::Unique(fs, s) => ("unique", fs, s),
                    RelationClause::Index {
                        interval: true,
                        fields,
                        scope,
                    } => ("interval index", fields, scope),
                    RelationClause::Index { fields, scope, .. } => ("index", fields, scope),
                    _ => continue,
                };
                let scope = match s {
                    None => None,
                    Some(s) => schema
                        .relation_sel(rel, &scope_sel(s))
                        .ok()
                        .and_then(|cs| schema.normalize_ctors(def, cs)),
                };
                out.push(ScopedClause {
                    decl: i,
                    range: ranges[k].clone(),
                    kind,
                    fields: fs
                        .iter()
                        .map(|e| {
                            text[e.span.start as usize - h..e.span.end as usize - h].to_string()
                        })
                        .collect(),
                    scope,
                });
            }
        }
        Ok(out)
    }

    /// Checks a routing `match` over rows of `sources` against products `products`, which a schema with
    /// both declares; returns what it carries whole into each product.
    pub(super) fn routing(
        &self,
        def_q: &QName,
        sources: &[String],
        products: &[String],
        product_texts: &[Written],
        arms: &[ast::Arm],
    ) -> R<Carries> {
        let rt = self.routing_types(def_q, sources, products, product_texts)?;
        let (schema, program, module, row, expected) =
            (&rt.schema, &rt.program, rt.module, &rt.row, &rt.result);
        let mut refs = Vec::new();
        check::check_row(
            schema,
            program,
            module,
            row,
            RowBody::Match(arms, self.span),
            Some(expected),
            Some(&mut refs),
        )
        .map_err(|d| vec![d])?;
        self.note(schema, &refs);
        let mut carries = Vec::new();
        for img in arms.iter().map(arm_image) {
            if let (Some(from), Some(to)) = (&img.from, &img.to) {
                for (g, f) in &img.given {
                    if let Some(f) = f {
                        carries.push((from.clone(), f.clone(), to.clone(), g.clone()));
                    }
                }
            }
        }
        Ok(carries)
    }

    /// Returns what a routing over rows of `sources` to `products` types against: the schema with
    /// the products appended to `def_q`, its row type over the sources and its result type over the
    /// products.
    pub(super) fn routing_types(
        &self,
        def_q: &QName,
        sources: &[String],
        products: &[String],
        product_texts: &[Written],
    ) -> R<RowTypes> {
        let (_, program) = self.checked()?;
        let schema = self.with_retyped_ctors(def_q, sources, products, product_texts)?;
        let def = self.type_id(&schema, def_q)?;
        let td = schema.ty(def);
        let src_ix: Vec<u32> = sources.iter().filter_map(|s| td.ctor_index(s)).collect();
        let prod_ix: Vec<u32> = products.iter().filter_map(|p| td.ctor_index(p)).collect();
        let module = schema
            .module_named(&self.module_of(def_q))
            .unwrap_or(schema::PRELUDE);
        let row = Self::row_ty(&schema, def, Some(src_ix));
        let result = Self::row_ty(&schema, def, Some(prod_ix));
        Ok(RowTypes {
            schema,
            program,
            module,
            row,
            result,
        })
    }

    /// The text of a `unique` or `index` clause of a split, with the fields as written in the migration.
    fn replacement_text(&self, kind: &str, fields: &[Expr], scope: &ast::CtorScope) -> Written {
        let names: Vec<String> = scope_idents(scope).iter().map(|x| x.text.clone()).collect();
        Written::concat([
            format!("{kind} (").into(),
            self.fields_text(fields),
            format!(") in {}", sel_text(&names)).into(),
        ])
    }

    /// The schema with constructors `texts` appended to type declaration `i`, beside the ones it has.
    pub(super) fn with_ctors(&self, i: usize, t: &ast::TypeDecl, texts: &[Written]) -> R<Schema> {
        Self::built(&self.ctors_appended(i, t, texts), self.span)
    }

    fn ctors_appended(&self, i: usize, t: &ast::TypeDecl, texts: &[Written]) -> Source {
        let mut temp = self.src.clone();
        let h = header(&temp.decls[i].module).len();
        let text = temp.decls[i].text.clone();
        let at = ctor_extents(&text, h, t).last().map_or(text.len(), |c| c.2);
        temp.edit(
            i,
            vec![(at..at, Written::prefixed(texts.iter().cloned(), "\n  | "))],
        );
        temp
    }

    /// The schema with constructors `texts` of `products` appended to type declaration `i`, and every
    /// reference to the constructors `sources` outside a check retyped to the products, theirs included.
    fn with_retyped_ctors(
        &self,
        def_q: &QName,
        sources: &[String],
        products: &[String],
        texts: &[Written],
    ) -> R<Schema> {
        let schema = self.schema()?;
        let def = self.type_id(&schema, def_q)?;
        let (i, t) = self.type_decl(def_q)?;
        let mut temp = self.ctors_appended(i, &t, texts);
        let edits = Cx {
            src: &mut temp,
            module: self.module.clone(),
            text: self.text,
            span: self.span,
            noted: None,
            copying: false,
            steps: None,
        }
        .reselect(
            &schema,
            def,
            sources,
            products,
            Reselect {
                whole: false,
                clauses: false,
                checks: false,
                what: "the routing",
            },
        )?;
        for (k, es) in edits {
            temp.edit(k, es);
        }
        Self::built(&temp, self.span)
    }

    fn built(src: &Source, span: Span) -> R<Schema> {
        src.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(span, d.diag.message))
                .collect()
        })
    }

    /// The edits that replace, in a type declaration, the constructors `sources` by `text`, at the first source's place.
    pub(super) fn replace_ctors(
        &self,
        i: usize,
        t: &ast::TypeDecl,
        sources: &[String],
        text: &Written,
    ) -> Edits {
        let h = header(&self.src.decls[i].module).len();
        let ctors = ctor_extents(&self.src.decls[i].text, h, t);
        let mut edits = Vec::new();
        let mut first = true;
        for (k, c) in ctors.iter().enumerate() {
            if !sources.contains(&c.0) {
                continue;
            }
            if first {
                edits.push((c.1.start..c.2, text.clone()));
                first = false;
            } else {
                edits.push((ctor_removal(&ctors, k), Written::default()));
            }
        }
        BTreeMap::from([(i, edits)])
    }

    /// The edits replacing, in each dependent an `arm` clause names, the arms over `replaced` by the clauses' arms.
    pub(super) fn arm_replacements(
        &self,
        schema: &Schema,
        refs: &[(FileId, Span, Ref)],
        def: TypeId,
        replaced: &[u32],
        clauses: &[ast::ArmClause],
    ) -> R<Edits> {
        let mut groups: BTreeMap<(usize, (u32, u32)), Vec<Written>> = BTreeMap::new();
        for a in clauses {
            let (k, within) = self.arm_target(a)?;
            let at = self.target_match(refs, a, k, within, &|ty| {
                replaced.iter().any(|&c| holds(schema, ty, def, c))
            })?;
            groups
                .entry((k, (at.start, at.end)))
                .or_default()
                .push(self.arm_text(a));
        }
        let mut edits: Edits = BTreeMap::new();
        for ((k, (s, e)), arms) in groups {
            let span = Span { start: s, end: e };
            edits
                .entry(k)
                .or_default()
                .extend(self.replace_arms(refs, k, span, def, replaced, &arms));
        }
        Ok(edits)
    }

    /// Moves the write states of `sources` to `products`. With `merge` a product is declared when a source is,
    /// else enabled, and a deprecated source refuses; otherwise it takes the first source's state.
    pub(super) fn restate(
        &mut self,
        schema: &Schema,
        def: TypeId,
        type_module: &str,
        sources: &[String],
        products: &[String],
        merge: bool,
    ) -> R<()> {
        let tname = schema.ty(def).name.clone();
        let combine = |states: Vec<Stage>| -> Result<Stage, Stage> {
            if !merge {
                return Ok(states.first().copied().unwrap_or(Stage::Enabled));
            }
            if states.contains(&Stage::Deprecated) {
                return Err(Stage::Deprecated);
            }
            Ok(if states.contains(&Stage::Declared) {
                Stage::Declared
            } else {
                Stage::Enabled
            })
        };
        let since = self.src.version;
        let mut keys: Vec<(String, String, bool)> = Self::relations_of(schema, def)
            .into_iter()
            .map(|(_, m, r)| (m, r, true))
            .collect();
        keys.push((type_module.to_string(), tname, false));
        for (m, r, relation) in keys {
            let map = if relation {
                &mut self.src.states
            } else {
                &mut self.src.value_states
            };
            let stages: Vec<Stage> = sources
                .iter()
                .map(|c| {
                    map.get(&(m.clone(), r.clone(), c.clone()))
                        .map_or(Stage::Enabled, |s| s.stage)
                })
                .collect();
            let Ok(stage) = combine(stages) else {
                let which = sources
                    .iter()
                    .find(|c| {
                        map.get(&(m.clone(), r.clone(), (*c).clone()))
                            .is_some_and(|s| s.stage == Stage::Deprecated)
                    })
                    .cloned()
                    .unwrap_or_default();
                return Err(vec![Diag::new(
                    self.span,
                    format!(
                        "merging or nesting a deprecated constructor is refused: `{r}.{which}` is deprecated"
                    ),
                )]);
            };
            for c in sources {
                map.remove(&(m.clone(), r.clone(), c.clone()));
            }
            for p in products {
                map.insert((m.clone(), r.clone(), p.clone()), State { stage, since });
            }
        }
        Ok(())
    }

    /// The sources a constructor selection of type `def` names, in declaration order.
    pub(super) fn sources(&self, schema: &Schema, def: TypeId, sel: &CtorSel) -> R<Vec<String>> {
        let td = schema.ty(def);
        let mut ix = Vec::new();
        for x in sel_idents(sel) {
            match td.ctor_index(&x.text) {
                Some(c) => ix.push(c),
                None => {
                    return self.err_at(
                        x.span,
                        format!("`{}` is not a constructor of `{}`", x.text, td.name),
                    );
                }
            }
        }
        ix.sort_unstable();
        ix.dedup();
        Ok(ix
            .iter()
            .map(|&c| td.ctors()[c as usize].name.clone())
            .collect())
    }

    pub(super) fn fresh_ctor(&self, schema: &Schema, def: TypeId, d: &ast::CtorDef) -> R<()> {
        if schema.ty(def).ctor_index(&d.name.text).is_some() {
            return self.err_at(
                d.name.span,
                format!(
                    "`{}` is already a constructor of `{}`",
                    d.name.text,
                    schema.ty(def).name
                ),
            );
        }
        Ok(())
    }

    /// Retypes the references to `sources` the products' own declarations still name.
    fn retype_products(
        &mut self,
        schema: &Schema,
        def: TypeId,
        sources: &[String],
        products: &[String],
    ) -> R<()> {
        let edits = self.reselect(
            schema,
            def,
            sources,
            products,
            Reselect {
                whole: false,
                clauses: false,
                checks: true,
                what: "the operator",
            },
        )?;
        self.apply_edits(edits);
        Ok(())
    }

    /// Merges the edit maps, dropping from `base` every edit inside a range `over` replaces.
    pub(super) fn merged(mut base: Edits, over: Edits) -> Edits {
        for (k, es) in over {
            let b = base.entry(k).or_default();
            for (r, _) in &es {
                if !r.is_empty() {
                    b.retain(|(x, _)| !(x.start >= r.start && x.end <= r.end && x != r));
                }
            }
            b.extend(es);
        }
        base
    }
}

/// Returns `names` with `by` in place of `gone`: `None` when it names none of them, and `Some(None)`
/// when `whole` asks for all or none and it names only some.
fn reselected(
    names: &[String],
    gone: &[String],
    by: &[String],
    whole: bool,
) -> Option<Option<Vec<String>>> {
    let hit = names.iter().filter(|n| gone.contains(n)).count();
    if hit == 0 {
        return None;
    }
    if whole && hit != gone.len() {
        return Some(None);
    }
    let mut out = Vec::new();
    for n in names {
        if gone.contains(n) {
            for b in by {
                if !out.contains(b) {
                    out.push(b.clone());
                }
            }
        } else {
            out.push(n.clone());
        }
    }
    Some(Some(out))
}

/// The bindings of a routing or fill arm's pattern: the constructor it matches, a binder of the
/// whole row, and each variable bound to a field, with that field.
struct ArmBinding {
    from: Option<String>,
    binder: Option<String>,
    bound: BTreeMap<String, String>,
}

impl ArmBinding {
    fn of(a: &ast::Arm) -> ArmBinding {
        let mut out = ArmBinding {
            from: None,
            binder: None,
            bound: BTreeMap::new(),
        };
        let Some(ast::Pattern {
            kind:
                ast::PatternKind::Ctor {
                    path,
                    ctor,
                    payload,
                },
            ..
        }) = a.patterns.first()
        else {
            return out;
        };
        out.from = Some(ctor.as_ref().unwrap_or(&path.name).text.clone());
        match payload {
            Some(ast::Payload::Bind(x)) => out.binder = Some(x.text.clone()),
            Some(ast::Payload::Record(fs)) => {
                for f in fs {
                    match &f.pattern {
                        None => {
                            out.bound.insert(f.field.text.clone(), f.field.text.clone());
                        }
                        Some(ast::Pattern {
                            kind: ast::PatternKind::Bind(x),
                            ..
                        }) => {
                            out.bound.insert(x.text.clone(), f.field.text.clone());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Returns the field of the matched row that `e` is, whole.
    fn carried(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ast::ExprKind::Field(x, f) => match &x.kind {
                ast::ExprKind::Name(q)
                    if self.from.is_some()
                        && q.module.is_none()
                        && Some(&q.name.text) == self.binder.as_ref() =>
                {
                    Some(f.text.clone())
                }
                _ => None,
            },
            ast::ExprKind::Name(q) if q.module.is_none() => self.bound.get(&q.name.text).cloned(),
            _ => None,
        }
    }
}

/// Returns the fields of the matched row that routing arms `arms` read to choose an arm: those
/// a pattern constrains, and every one of `fields`, the header's included, when an arm has a
/// guard.
pub(super) fn matched_fields(arms: &[ast::Arm], fields: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for a in arms {
        if a.guard.is_some() {
            out.extend(fields.iter().cloned());
        }
        for p in &a.patterns {
            if let ast::PatternKind::Ctor {
                payload: Some(ast::Payload::Record(fs)),
                ..
            } = &p.kind
            {
                out.extend(
                    fs.iter()
                        .filter(|f| {
                            f.pattern
                                .as_ref()
                                .is_some_and(|x| !matches!(x.kind, ast::PatternKind::Bind(_)))
                        })
                        .map(|f| f.field.text.clone()),
                );
            }
        }
    }
    out.into_iter().collect()
}

/// Returns the constructor fill arm `a` matches, and the field of it its value is whole.
pub(super) fn fill_image(a: &ast::Arm) -> (Option<String>, Option<String>) {
    let b = ArmBinding::of(a);
    let carried = if a.guard.is_none() {
        b.carried(&a.body)
    } else {
        None
    };
    (b.from, carried)
}

/// Returns what routing arm `a` matches, makes, and carries whole.
pub(super) fn arm_image(a: &ast::Arm) -> carry::ArmImage {
    let b = ArmBinding::of(a);
    let (to, given) = match &a.body.kind {
        ast::ExprKind::Ctor {
            path: bp,
            ctor: bc,
            record,
        } => {
            let to = bc.as_ref().unwrap_or(&bp.name).text.clone();
            match record {
                None => (Some(to), Vec::new()),
                Some(ast::RecordExpr::Fields(given, _)) => (
                    Some(to),
                    given
                        .iter()
                        .map(|(g, e)| (g.text.clone(), b.carried(e)))
                        .collect(),
                ),
                Some(_) => (None, Vec::new()),
            }
        }
        _ => (None, Vec::new()),
    };
    carry::ArmImage {
        from: b.from,
        to,
        given,
    }
}
