use super::*;

/// The relations over a type, each as its id, module and name.
type Rels = [(RelId, String, String)];

impl Cx<'_> {
    pub(in crate::evolve) fn remove_constructor(&mut self, path: &ast::CtorPath) -> R<()> {
        self.remove_ctor(path, false)
    }

    /// Removes a constructor; as the inverse of `add constructor` it skips the lifecycle check and removes
    /// the constructor's arms from every match.
    pub(in crate::evolve) fn remove_ctor(&mut self, path: &ast::CtorPath, inverse: bool) -> R<()> {
        let schema = self.schema()?;
        let (i, t, def, c) = self.ctor_of(&schema, path)?;
        let name = path.ctor.text.clone();
        let rels = Self::relations_of(&schema, def);
        let why = if inverse {
            None
        } else {
            self.not_deprecated(path, &t, &rels)
        };
        if let Some(why) = why.or_else(|| removal_blocked(&schema, &t, &name, def, c, &rels)) {
            let shown = format!("{}.{name}", t.name.text);
            return self.err(format!("`remove constructor {shown}` is refused: {why}"));
        }
        let mut edits = self.reselect(
            &schema,
            def,
            std::slice::from_ref(&name),
            &[],
            Reselect {
                whole: false,
                clauses: true,
                checks: true,
                what: "the remove",
            },
        )?;
        self.clause_removals(&schema, &rels, c, &name, &mut edits);
        self.arm_removals(&schema, def, c, inverse, &mut edits);
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
        self.forget_states(path, &t, &rels);
        Ok(())
    }

    /// Returns why constructor `path` may not be removed yet: it is not deprecated, by an earlier
    /// migration, in its type or in a relation over it.
    fn not_deprecated(
        &self,
        path: &ast::CtorPath,
        t: &ast::TypeDecl,
        rels: &Rels,
    ) -> Option<String> {
        let name = &path.ctor.text;
        let version = self.src.version;
        let earlier = |s: Option<&State>| {
            s.is_some_and(|s| s.stage == Stage::Deprecated && s.since < version)
        };
        if rels.is_empty() {
            let key = (self.module_of(&path.ty), t.name.text.clone(), name.clone());
            if !earlier(self.src.value_states.get(&key)) {
                return Some(format!(
                    "`{name}` is not deprecated in its type by an earlier migration"
                ));
            }
        }
        rels.iter()
            .find(|(_, m, r)| !earlier(self.src.states.get(&(m.clone(), r.clone(), name.clone()))))
            .map(|(_, _, r)| format!("`{name}` is not deprecated in `{r}` by an earlier migration"))
    }

    /// Remove every clause of a relation over the type that ranges over constructor `c` alone.
    fn clause_removals(&self, schema: &Schema, rels: &Rels, c: u32, name: &str, edits: &mut Edits) {
        for (rel, m, rname) in rels {
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
    }

    /// Remove the arms for constructor `c` from the matches of constraints, or of every
    /// declaration when `inverse`.
    fn arm_removals(&self, schema: &Schema, def: TypeId, c: u32, inverse: bool, edits: &mut Edits) {
        let refs = check::schema_refs(schema);
        for j in 0..self.src.decls.len() {
            if !inverse && !matches!(self.src.parsed(j), Some(Decl::Constraint(_))) {
                continue;
            }
            let matches: Vec<Span> = refs
                .iter()
                .filter(|(f, _, r)| {
                    f.0 as usize == j && matches!(r, Ref::Match(ty) if holds(schema, ty, def, c))
                })
                .map(|(_, s, _)| *s)
                .collect();
            for at in matches {
                let mut es = self.replace_arms(&refs, j, at, def, &[c], &[]);
                es.retain(|(r, s)| !(r.is_empty() && s.text.is_empty()));
                edits.entry(j).or_default().extend(es);
            }
        }
    }

    /// Forget the write states of constructor `path` in its type and in every relation over it.
    fn forget_states(&mut self, path: &ast::CtorPath, t: &ast::TypeDecl, rels: &Rels) {
        let name = &path.ctor.text;
        let rel_keys: BTreeSet<(String, String)> = rels
            .iter()
            .map(|(_, m, r)| (m.clone(), r.clone()))
            .collect();
        self.src
            .states
            .retain(|(m, r, x), _| !(rel_keys.contains(&(m.clone(), r.clone())) && x == name));
        let tm = self.module_of(&path.ty);
        self.src
            .value_states
            .retain(|(m, ty, x), _| !(*m == tm && *ty == t.name.text && x == name));
    }
}

/// Returns why removing constructor `c` of type `def` would break the schema.
fn removal_blocked(
    schema: &Schema,
    t: &ast::TypeDecl,
    name: &str,
    def: TypeId,
    c: u32,
    rels: &Rels,
) -> Option<String> {
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
                    return Some(format!(
                        "field `{}.{}` references a constructor set that holds it",
                        u.name, f.name
                    ));
                }
            }
        }
    }
    for (rel, _, r) in rels {
        for s in &schema.relation(*rel).sets {
            if s.ctors == [c] {
                return Some(format!(
                    "the named set `{}` of `{r}` would become empty",
                    s.name
                ));
            }
        }
    }
    if schema.ty(def).ctors().len() == 1 {
        return Some(format!(
            "`{name}` is the only constructor of `{}`",
            t.name.text
        ));
    }
    schema
        .checks
        .iter()
        .find(|chk| {
            rels.iter().any(|(r, _, _)| *r == chk.relation)
                && chk.ctors.as_ref().is_some_and(|cs| cs == &[c])
        })
        .map(|chk| {
            format!(
                "check `{}` requires rows of `{name}` and has no image without it",
                chk.name
            )
        })
}
