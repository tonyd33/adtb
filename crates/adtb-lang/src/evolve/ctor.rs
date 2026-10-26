use super::*;

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
        let end = at.end as usize - h;
        edits.push((end..end, Written::prefixed(arms.iter().cloned(), "\n  | ")));
        edits
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
            ast::TypeBody::Record(_) => {
                let h = header(&self.src.decls[i].module).len();
                let text = self.src.decls[i].text.clone();
                let Some(braces) = field_braces(&text, h, &t, None) else {
                    return self.err_at(
                        path.ty.span(),
                        format!("record `{tname}` has no fields to keep"),
                    );
                };
                self.src
                    .edit(i, vec![(braces.start..braces.start, format!("{tname} "))]);
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
            let end = at.end as usize - header(&self.src.decls[k].module).len();
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
            what,
        } = mode;
        let mut edits: Edits = BTreeMap::new();
        let mut errs = Vec::new();
        let rewrite = |names: &[String]| -> Option<Option<Vec<String>>> {
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
        };
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
                    if let ast::TypeKind::Key(rr) = &t.kind
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
                    Decl::Check(c) if names_rel(&c.on.relation) => {
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
                let h = header(&self.src.decls[j].module).len();
                let text = &self.src.decls[j].text;
                let from = idents[0].span.start as usize - h;
                let to = idents.last().expect("an ident").span.end as usize - h;
                let (from, to) = if idents.len() > 1 || text[..from].trim_end().ends_with('(') {
                    let open = text[..from].trim_end().len() - 1;
                    let close = to + text[to..].find(')').map_or(0, |k| k + 1);
                    (open, close)
                } else {
                    (from, to)
                };
                edits
                    .entry(j)
                    .or_default()
                    .push((from..to, sel_text(&out).into()));
            }
        }
        if errs.is_empty() {
            Ok(edits)
        } else {
            Err(errs)
        }
    }

    pub(super) fn remove_constructor(&mut self, path: &ast::CtorPath) -> R<()> {
        self.remove_ctor(path, false)
    }

    /// Removes a constructor; as the inverse of `add constructor` it skips the lifecycle check and removes
    /// the constructor's arms from every match.
    #[expect(
        clippy::too_many_lines,
        reason = "the edits to relations, types, sets, checks and match arms would each have to be a step"
    )]
    pub(super) fn remove_ctor(&mut self, path: &ast::CtorPath, inverse: bool) -> R<()> {
        let schema = self.schema()?;
        let (i, t, def, c) = self.ctor_of(&schema, path)?;
        let name = path.ctor.text.clone();
        let shown = format!("{}.{name}", t.name.text);
        let refused = |why: String| -> R<()> {
            self.err(format!("`remove constructor {shown}` is refused: {why}"))
        };
        let rels = Self::relations_of(&schema, def);
        let version = self.src.version;
        let earlier = |s: Option<&State>| {
            s.is_some_and(|s| s.stage == Stage::Deprecated && s.since < version)
        };
        if rels.is_empty() && !inverse {
            let key = (self.module_of(&path.ty), t.name.text.clone(), name.clone());
            if !earlier(self.src.value_states.get(&key)) {
                return refused(format!(
                    "`{name}` is not deprecated in its type by an earlier migration"
                ));
            }
        }
        for (_, m, r) in rels.iter().filter(|_| !inverse) {
            if !earlier(self.src.states.get(&(m.clone(), r.clone(), name.clone()))) {
                return refused(format!(
                    "`{name}` is not deprecated in `{r}` by an earlier migration"
                ));
            }
        }
        for u in &schema.types {
            for f in u
                .header()
                .iter()
                .chain(u.ctors().iter().flat_map(|x| x.fields.iter()))
            {
                let mut keys = Vec::new();
                key_types(&f.ty, &mut keys);
                for (rel, ctors) in keys {
                    if schema.relation(rel).elem_def == def
                        && let Some(cs) = ctors
                        && cs.contains(&c)
                    {
                        return refused(format!(
                            "field `{}.{}` references a constructor set that holds it",
                            u.name, f.name
                        ));
                    }
                }
            }
        }
        for (rel, _, r) in &rels {
            for s in &schema.relation(*rel).sets {
                if s.ctors == [c] {
                    return refused(format!(
                        "the named set `{}` of `{r}` would become empty",
                        s.name
                    ));
                }
            }
        }
        if schema.ty(def).ctors().len() == 1 {
            return refused(format!(
                "`{name}` is the only constructor of `{}`",
                t.name.text
            ));
        }
        for chk in &schema.checks {
            if rels.iter().any(|(r, _, _)| *r == chk.relation)
                && chk.ctors.as_ref().is_some_and(|cs| cs == &[c])
            {
                return refused(format!(
                    "check `{}` requires rows of `{name}` and has no image without it",
                    chk.name
                ));
            }
        }
        let refs = check::schema_refs(&schema);
        let mut edits = self.reselect(
            &schema,
            def,
            std::slice::from_ref(&name),
            &[],
            Reselect {
                whole: false,
                clauses: true,
                what: "the remove",
            },
        )?;
        for (rel, m, rname) in &rels {
            let Some(k) = self.src.find("relation", m, rname) else {
                continue;
            };
            let Some(Decl::Relation(rd)) = self.src.parsed(k) else {
                continue;
            };
            let text = self.src.decls[k].text.clone();
            let h = header(m).len();
            let Some(ranges) = clause_ranges(&text, h, &rd) else {
                continue;
            };
            for (n, cl) in rd.clauses.iter().enumerate() {
                let only = match cl {
                    RelationClause::Unique(_, Some(s))
                    | RelationClause::Index { scope: Some(s), .. } => schema
                        .relation_sel(*rel, &scope_sel(s))
                        .ok()
                        .is_some_and(|cs| cs == [c]),
                    RelationClause::Default { ctor: Some(x), .. } => x.text == name,
                    _ => false,
                };
                if only {
                    let e = edits.entry(k).or_default();
                    e.retain(|(r, _)| !(r.start >= ranges[n].start && r.end <= ranges[n].end));
                    e.push((clause_removal(&text, &ranges, n), String::new().into()));
                }
            }
        }
        for j in 0..self.src.decls.len() {
            if !inverse && !matches!(self.src.parsed(j), Some(Decl::Constraint(_))) {
                continue;
            }
            let matches: Vec<Span> = refs
                .iter()
                .filter(|(f, _, r)| {
                    f.0 as usize == j && matches!(r, Ref::Match(ty) if holds(&schema, ty, def, c))
                })
                .map(|(_, s, _)| *s)
                .collect();
            for at in matches {
                let mut es = self.replace_arms(&refs, j, at, def, &[c], &[]);
                es.retain(|(r, s)| !(r.is_empty() && s.text.is_empty()));
                edits.entry(j).or_default().extend(es);
            }
        }
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let ctors = ctor_extents(&text, h, &t);
        let k = ctors
            .iter()
            .position(|x| x.0 == name)
            .expect("the constructor");
        edits
            .entry(i)
            .or_default()
            .push((ctor_removal(&ctors, k), String::new().into()));
        self.apply_edits(edits);
        let rel_keys: BTreeSet<(String, String)> = rels
            .iter()
            .map(|(_, m, r)| (m.clone(), r.clone()))
            .collect();
        self.src
            .states
            .retain(|(m, r, x), _| !(rel_keys.contains(&(m.clone(), r.clone())) && *x == name));
        let tm = self.module_of(&path.ty);
        self.src
            .value_states
            .retain(|(m, ty, x), _| !(*m == tm && *ty == t.name.text && *x == name));
        Ok(())
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
            if let Some(ast::RefAction::Retag(x, _)) = &f.on_delete
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
        let (_, program) = self.checked()?;
        let (i, t) = self.type_decl(def_q)?;
        let schema = self.with_ctors(i, &t, product_texts)?;
        let def = self.type_id(&schema, def_q)?;
        let td = schema.ty(def);
        let src_ix: Vec<u32> = sources.iter().filter_map(|s| td.ctor_index(s)).collect();
        let prod_ix: Vec<u32> = products.iter().filter_map(|p| td.ctor_index(p)).collect();
        let module = schema
            .module_named(&self.module_of(def_q))
            .unwrap_or(schema::PRELUDE);
        let row = Self::row_ty(&schema, def, Some(src_ix));
        let expected = Self::row_ty(&schema, def, Some(prod_ix));
        let mut refs = Vec::new();
        check::check_row(
            &schema,
            &program,
            module,
            &row,
            RowBody::Match(arms, self.span),
            Some(&expected),
            Some(&mut refs),
        )
        .map_err(|d| vec![d])?;
        self.note(&schema, &refs);
        let mut carries = Vec::new();
        for a in arms {
            let Some(p) = a.patterns.first() else {
                continue;
            };
            let ast::PatternKind::Ctor {
                path,
                ctor,
                payload,
            } = &p.kind
            else {
                continue;
            };
            let from = ctor.as_ref().unwrap_or(&path.name).text.clone();
            let mut bound: BTreeMap<String, String> = BTreeMap::new();
            let mut binder = None;
            match payload {
                Some(ast::Payload::Bind(x)) => binder = Some(x.text.clone()),
                Some(ast::Payload::Record(fs)) => {
                    for f in fs {
                        match &f.pattern {
                            None => {
                                bound.insert(f.field.text.clone(), f.field.text.clone());
                            }
                            Some(ast::Pattern {
                                kind: ast::PatternKind::Bind(x),
                                ..
                            }) => {
                                bound.insert(x.text.clone(), f.field.text.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            let ast::ExprKind::Ctor {
                path: bp,
                ctor: bc,
                record: Some(ast::RecordExpr::Fields(given, _)),
            } = &a.body.kind
            else {
                continue;
            };
            let to = bc.as_ref().unwrap_or(&bp.name).text.clone();
            for (g, e) in given {
                let carried = match &e.kind {
                    ast::ExprKind::Field(x, f) => match &x.kind {
                        ast::ExprKind::Name(q)
                            if q.module.is_none() && Some(&q.name.text) == binder.as_ref() =>
                        {
                            Some(f.text.clone())
                        }
                        _ => None,
                    },
                    ast::ExprKind::Name(q) if q.module.is_none() => {
                        bound.get(&q.name.text).cloned()
                    }
                    _ => None,
                };
                if let Some(f) = carried {
                    carries.push((from.clone(), f, to.clone(), g.text.clone()));
                }
            }
        }
        Ok(carries)
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
        let mut temp = self.src.clone();
        let h = header(&temp.decls[i].module).len();
        let text = temp.decls[i].text.clone();
        let at = ctor_extents(&text, h, t).last().map_or(text.len(), |c| c.2);
        temp.edit(
            i,
            vec![(at..at, Written::prefixed(texts.iter().cloned(), "\n  | "))],
        );
        temp.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(self.span, d.diag.message))
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
    ) -> Vec<(Range<usize>, Written)> {
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
        edits
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

    #[expect(
        clippy::too_many_lines,
        reason = "routing the arms, choosing the discriminator and rewriting checks and clauses would each have to be a step"
    )]
    pub(super) fn merge(
        &mut self,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: Option<&[ast::Arm]>,
        clauses: &[ast::ArmClause],
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
        let disc = match discs.as_slice() {
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
        let carries: Carries = match (arms, carries) {
            (Some(a), Some(c)) => {
                for arm in a {
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
                                g.text == disc
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
                c
            }
            _ => {
                let td = schema.ty(def);
                let mut c = Vec::new();
                for s in &sources {
                    let k = td.ctor_index(s).expect("a source");
                    let sf = &td.ctors()[k as usize].fields;
                    for f in sf {
                        match mfields.iter().find(|g| g.name == f.name && g.name != disc) {
                            Some(g) if g.ty == f.ty => {
                                c.push((s.clone(), f.name.clone(), m.clone(), f.name.clone()));
                            }
                            _ => {
                                return self.err(format!(
                                    "field `{s}.{}` has no counterpart in `{m}`",
                                    f.name
                                ));
                            }
                        }
                    }
                    for g in &mfields {
                        if g.name != disc && !sf.iter().any(|f| f.name == g.name) {
                            return self.err(format!(
                                "field `{m}.{}` has no counterpart in `{s}`",
                                g.name
                            ));
                        }
                    }
                }
                c
            }
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
                what: "the merge",
            },
        )?;
        for chk in &schema.checks {
            if let Some(cs) = &chk.ctors
                && cs.iter().any(|c| src_ix.contains(c))
                && !src_ix.iter().all(|c| cs.contains(c))
            {
                return self.err(format!(
                    "check `{}` has no image under the merge; drop or rewrite it in this or an earlier migration",
                    chk.name
                ));
            }
        }
        let mut clause_edits: Edits = BTreeMap::new();
        let mut added: BTreeMap<usize, Vec<Written>> = BTreeMap::new();
        for cl in self.scoped_clauses(&schema, def)? {
            let Some(scope) = &cl.scope else { continue };
            let inside = scope.iter().filter(|c| src_ix.contains(c)).count();
            if inside == 0 {
                continue;
            }
            let shown = format!("{} ({})", cl.kind, cl.fields.join(", "));
            let text = self.src.decls[cl.decl].text.clone();
            if inside != scope.len() {
                if inside != src_ix.len() {
                    return self.err(format!("`{shown}` ranges over some but not all of the merged constructors and has no image under the merge"));
                }
                let td = schema.ty(def);
                let mut names: Vec<String> = Vec::new();
                for &c in scope {
                    let n = if src_ix.contains(&c) {
                        m.clone()
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
            let mut out_fields: Option<Vec<String>> = None;
            for &c in scope {
                let s = &schema.ty(def).ctors()[c as usize].name;
                let mut fs = Vec::new();
                for f in &cl.fields {
                    let header = schema.ty(def).header().iter().any(|x| x.name == *f);
                    let mapped = if header {
                        Some(f.clone())
                    } else {
                        carries
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
                if cl.kind == "unique" && !fs.contains(&disc) {
                    fs.insert(0, disc.clone());
                }
                match &out_fields {
                    None => out_fields = Some(fs),
                    Some(prev) if *prev == fs => {}
                    Some(_) => {
                        return self.err(format!("`{shown}` has no image under the merge: its constructors' fields go to different fields"));
                    }
                }
            }
            clause_edits.entry(cl.decl).or_default().push((
                clause_removal(&text, std::slice::from_ref(&cl.range), 0),
                String::new().into(),
            ));
            let new = format!(
                "{} ({}) in {m}",
                cl.kind,
                out_fields.unwrap_or_default().join(", ")
            );
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
        edits = Self::merged(edits, clause_edits);
        edits
            .entry(i)
            .or_default()
            .extend(self.replace_ctors(i, &t, &sources, &m_text));
        let arm_edits = self.arm_replacements(&schema, &refs, def, &src_ix, clauses)?;
        edits = Self::merged(edits, arm_edits);
        self.apply_edits(edits);
        Ok(())
    }
}

impl Cx<'_> {
    #[expect(
        clippy::too_many_lines,
        reason = "rewriting the clauses, products and images of the split would each have to be a step"
    )]
    pub(super) fn split(
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
                ast::SplitClause::Arm(a) => arm_clauses.push(a.clone()),
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
                what: "the split",
            },
        )?;
        let mut clause_edits: Edits = BTreeMap::new();
        let mut added: BTreeMap<usize, Vec<Written>> = BTreeMap::new();
        let mut decls_of_t = BTreeSet::new();
        for cl in self.scoped_clauses(&schema, def)? {
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
                        names.extend(products.iter().cloned());
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
            let mut images = Vec::new();
            let mut lost = None;
            for p in &products {
                let mut fs = Vec::new();
                for f in &cl.fields {
                    if header.contains(f.as_str()) {
                        fs.push(f.clone());
                    } else if let Some((_, _, _, g)) = carries
                        .iter()
                        .find(|(x, y, z, _)| *x == source && y == f && z == p)
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
            for f in &cl.fields {
                let kept = header.contains(f.as_str())
                    || carries.iter().any(|(x, y, _, _)| *x == source && y == f);
                if !kept {
                    lost = Some(f.clone());
                }
            }
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
            for img in images {
                if !list.iter().any(|x| x.text == img) {
                    list.push(img.into());
                }
            }
        }
        for k in decls_of_t {
            let list = added.entry(k).or_default();
            for r in &replacements {
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
        edits = Self::merged(edits, clause_edits);
        edits.entry(i).or_default().extend(self.replace_ctors(
            i,
            &t,
            std::slice::from_ref(&source),
            &Written::joined(texts, "\n  | "),
        ));
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
        self.apply_edits(edits);
        Ok(())
    }

    pub(super) fn nest(
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
        edits
            .entry(i)
            .or_default()
            .extend(self.replace_ctors(i, &t, &sources, &n_text));
        let arm_edits = self.arm_replacements(&schema, &refs, def, &src_ix, clauses)?;
        edits = Self::merged(edits, arm_edits);
        self.apply_edits(edits);
        Ok(())
    }

    pub(super) fn unnest(&mut self, path: &ast::CtorPath, clauses: &[ast::ArmClause]) -> R<()> {
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
                what: "the unnest",
            },
        )?;
        let mut clause_edits: Edits = BTreeMap::new();
        let header: BTreeSet<&str> = td.header().iter().map(|f| f.name.as_str()).collect();
        for cl in self.scoped_clauses(&schema, def)? {
            let Some(scope) = &cl.scope else { continue };
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
                Some(p) if scope.len() == 1 => vec![p.clone()],
                Some(_) => return self.err(format!("`{shown}` has no image under the unnest")),
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
        edits.entry(i).or_default().extend(self.replace_ctors(
            i,
            &t,
            std::slice::from_ref(&n),
            &texts.join("\n  | ").into(),
        ));
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
        self.apply_edits(edits);
        Ok(())
    }
}
