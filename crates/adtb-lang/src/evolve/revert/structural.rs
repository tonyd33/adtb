use super::inverse::Inverting;
use super::*;
use crate::ids::{KeyKind, Owner, Space};

/// How a structural operator's inverse applies its operator.
#[derive(Clone, Debug)]
pub(in crate::evolve) enum Undo {
    /// As the operator written.
    Written,
    /// A merge whose arms' fixed values tell its constructors apart.
    Derived,
    /// A move to the header of the field of every constructor of the ids.
    UpFrom(Vec<CatCtorId>),
    /// An unnest, then a rename of each constructor of the type of the id that it brings up from
    /// the product of an id to the name beside it.
    Unnest(CatTypeId, Vec<(CatCtorId, String)>),
}

/// A unique or index clause of a relation: its id, the declaration, its position among the
/// declaration's clauses and the range of its text.
type IndexClause = (u64, usize, usize, Range<usize>);

/// Returns the declaration of relation `rel` of `schema`, the schema of `src`, parsed, with the
/// range of each of its clauses.
fn relation_clauses(
    src: &Source,
    schema: &Schema,
    rel: RelId,
) -> Option<(usize, ast::RelationDecl, Vec<Range<usize>>)> {
    let rd = schema.relation(rel);
    let m = &schema.module(rd.module).name;
    let i = src.find("relation", m, &rd.name)?;
    let Some(Decl::Relation(d)) = src.parsed(i) else {
        return None;
    };
    let ranges = clause_ranges(&src.decls[i].text, header(m).len(), &d)?;
    Some((i, d, ranges))
}

/// Returns each unique and index clause of relation `rel` of `schema`, the schema of `src`, in
/// written order.
fn index_clauses(src: &Source, schema: &Schema, rel: RelId) -> Vec<IndexClause> {
    let Some((i, d, ranges)) = relation_clauses(src, schema, rel) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut at = 0;
    for (k, cl) in d.clauses.iter().enumerate() {
        if !matches!(
            cl,
            RelationClause::Unique(..) | RelationClause::Index { .. }
        ) {
            continue;
        }
        if let Some(id) = schema.ids().index_id(rel, at) {
            out.push((id.raw(), i, k, ranges[k].clone()));
        }
        at += 1;
    }
    out
}

/// Returns relation `rel` of `schema` qualified by its module, held by its id.
fn relation_spelled(schema: &Schema, rel: RelId) -> Spelled {
    let rd = schema.relation(rel);
    let mut out = Spelled::from(format!("{}::", schema.module(rd.module).name));
    let id = schema.ids().relation_id(rel).map(CatRelId::raw);
    out.name(&rd.name, Space::Shared, id);
    out
}

/// Returns type `t` of module `m` qualified by its module, held by its id in `src`.
fn type_spelled(src: &Source, m: &str, t: &str) -> Spelled {
    let mut out = Spelled::from(format!("{m}::"));
    let key = ObjectKey::named(KeyKind::Type, Owner::Module(m.to_string()), t);
    out.name(t, Space::Shared, src.ids.get(&key).copied());
    out
}

/// Returns constructor `c` of type `t` of module `m`, held by its id in `src`.
fn ctor_spelled(src: &Source, m: &str, t: &str, c: &str) -> Spelled {
    let mut out = Spelled::default();
    out.name(c, Space::Shared, src.ctor_key_id(m, t, c));
    out
}

impl Inverting<'_> {
    /// Returns each unique and index the operator created over a relation of type `t` of module
    /// `m`, and the operators that add back each it removed, each on a line after a line break.
    fn clauses_back(&self, m: &str, t: &str) -> R<(Vec<CatIndexId>, Spelled)> {
        let (before, after) = (self.before(), self.after());
        let replays = || vec![Diag::new(self.op().span, "the log does not replay")];
        let bs = before.schema().map_err(|_| replays())?;
        let r#as = after.schema().map_err(|_| replays())?;
        let def = bs
            .module_named(m)
            .and_then(|mid| bs.module(mid).types.get(t).copied())
            .ok_or_else(|| vec![Diag::new(self.op().span, "the log does not hold the type")])?;
        let mut drops = Vec::new();
        let mut adds = Spelled::default();
        for (rel, ..) in Cx::relations_of(&bs, def) {
            let was = index_clauses(before, &bs, rel);
            let Some(ar) = bs
                .ids()
                .relation_id(rel)
                .and_then(|id| r#as.ids().relation_of(id))
            else {
                continue;
            };
            let now = index_clauses(after, &r#as, ar);
            for (id, ..) in &now {
                if !was.iter().any(|c| c.0 == *id) {
                    drops.push(CatIndexId::from_raw(*id));
                }
            }
            for (id, i, _, range) in &was {
                if !now.iter().any(|c| c.0 == *id) {
                    let text = &before.decls[*i].text;
                    adds.push("\n");
                    adds.append(clause_op(
                        "add",
                        text,
                        range,
                        &relation_spelled(&bs, rel),
                        |r| self.decl_range(*i, r),
                    )?);
                }
            }
        }
        Ok((drops, adds))
    }

    /// Returns the `arm` clauses that put back, for each declaration an `arm` of `clauses`
    /// names, its arms over a constructor of type `ty` named in `ctors` as `before` declares
    /// them.
    fn restored_arms(
        &self,
        clauses: &[ast::ArmClause],
        ty: &QName,
        ctors: &[String],
    ) -> R<Spelled> {
        let before = self.before();
        let mut out = Spelled::default();
        if clauses.is_empty() {
            return Ok(out);
        }
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.op().span, "the log does not replay")])?;
        let def = schema
            .module_named(&self.module_of(ty))
            .and_then(|mid| schema.module(mid).types.get(&ty.name.text).copied())
            .ok_or_else(|| vec![Diag::new(self.op().span, "the log does not hold the type")])?;
        let cs: Vec<u32> = ctors
            .iter()
            .filter_map(|c| schema.ty(def).ctor_index(c))
            .collect();
        let refs = check::schema_refs(&schema);
        let mut seen = BTreeSet::new();
        for a in clauses {
            if !seen.insert(a.target.name.text.clone()) {
                continue;
            }
            let m = self.module_of(&a.target);
            for kind in ["fn", "constraint"] {
                let Some(i) = before.find(kind, &m, &a.target.name.text) else {
                    continue;
                };
                let Some(d) = before.parsed(i) else { continue };
                let h = header(&m).len();
                let mut ranges = Vec::new();
                for e in decl_exprs(&d) {
                    crate::visit::each_expr(e, &mut |x| {
                        if let ast::ExprKind::Match(_, arms) = &x.kind {
                            for arm in arms {
                                if arm_ctor(&refs, i, def, arm).is_some_and(|c| cs.contains(&c)) {
                                    let start = arm.patterns[0].span.start as usize - h;
                                    ranges.push(start..arm.body.span.end as usize - h);
                                }
                            }
                        }
                    });
                }
                for r in ranges {
                    out.push("\n  arm ");
                    out.append(self.qualified(&a.target, a.target.name.span.end as usize)?);
                    out.push(": ");
                    out.append(self.decl_range(i, r)?);
                }
            }
        }
        Ok(out)
    }

    /// Returns the text of constructor `c` of type `ty` as `before` declares it, with its field
    /// names.
    fn before_ctor(&self, ty: &QName, c: &str) -> R<(Spelled, Vec<String>)> {
        let before = self.before();
        let m = self.module_of(ty);
        let missing = || {
            vec![Diag::new(
                self.op().span,
                "the log does not hold the constructor",
            )]
        };
        let i = before.find("type", &m, &ty.name.text).ok_or_else(missing)?;
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return Err(missing());
        };
        let ctors = ctor_extents(&before.decls[i].text, header(&m).len(), &t);
        let k = ctors.iter().position(|x| x.0 == c).ok_or_else(missing)?;
        let text = self.decl_range(i, ctors[k].1.start..ctors[k].2)?;
        let fields = decl_fields(&t)
            .into_iter()
            .filter(|(x, _)| *x == Some(c))
            .map(|(_, f)| f.name.text.clone())
            .collect();
        Ok((text, fields))
    }

    /// Returns the step applying `op`, the inverse operator, as `how` says, after dropping the
    /// clauses the operator created over the relations of type `ty` and before adding back
    /// those it removed.
    fn restructure(
        &self,
        ty: &QName,
        op: Spelled,
        how: Undo,
        checks: Vec<(CatConstraintId, Spelled)>,
    ) -> R<Inverse> {
        let (drops, adds) = self.clauses_back(&self.module_of(ty), &ty.name.text)?;
        Ok(Inverse::Restructure {
            drops,
            op,
            adds,
            how,
            checks,
            of: self.of(),
        })
    }
}

/// Returns operator `verb` of the clause at `range` of `text`, on relation `rel`, its text copied
/// by `copy`.
fn clause_op(
    verb: &str,
    text: &str,
    range: &Range<usize>,
    rel: &Spelled,
    copy: impl Fn(Range<usize>) -> R<Spelled>,
) -> R<Spelled> {
    let clause = text[range.clone()].trim_end();
    let end = range.start + clause.len();
    let mut out = Spelled::from(format!("{verb} "));
    match clause.rfind(") in ") {
        Some(p) => {
            out.append(copy(range.start..range.start + p + 1)?);
            out.push(" in ");
            out.append(rel.clone());
            out.push(".");
            out.append(copy(range.start + p + 5..end)?);
        }
        None => {
            out.append(copy(range.start..end)?);
            out.push(" in ");
            out.append(rel.clone());
        }
    }
    Ok(out)
}

impl Cx<'_> {
    pub(super) fn split_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
        into: &[ast::CtorDef],
        arms: &[ast::Arm],
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let lossy = || {
            vec![Diag::new(
                self.span,
                format!(
                    "the split of `{}` routes rows by a routing that is not invertible, so it has no inverse",
                    path.ctor.text
                ),
            )]
        };
        let reached: BTreeSet<&str> = arms
            .iter()
            .filter_map(|a| match &a.body.kind {
                ast::ExprKind::Ctor { path, ctor, .. } => {
                    Some(ctor.as_ref().unwrap_or(&path.name).text.as_str())
                }
                _ => None,
            })
            .collect();
        if let Some(d) = into
            .iter()
            .find(|d| !reached.contains(d.name.text.as_str()))
        {
            return self.err(format!(
                "the split of `{}` routes no row to `{}`, so it is lossy and has no inverse",
                path.ctor.text, d.name.text
            ));
        }
        let (c_text, c_fields) = at.before_ctor(&path.ty, &path.ctor.text)?;
        let mut inverse_arms = Spelled::default();
        for a in arms {
            let Some(ast::Pattern {
                kind: ast::PatternKind::Ctor { payload, .. },
                ..
            }) = a.patterns.first()
            else {
                return Err(lossy());
            };
            let ast::ExprKind::Ctor {
                path: bp,
                ctor: bc,
                record,
            } = &a.body.kind
            else {
                return Err(lossy());
            };
            let product = bc.as_ref().unwrap_or(&bp.name).text.clone();
            let given: Vec<(String, &Expr)> = match record {
                Some(ast::RecordExpr::Fields(fs, _)) => {
                    fs.iter().map(|(g, e)| (g.text.clone(), e)).collect()
                }
                None => Vec::new(),
                _ => return Err(lossy()),
            };
            let mut vars: BTreeMap<String, String> = BTreeMap::new();
            let mut fixed: BTreeMap<String, Span> = BTreeMap::new();
            let mut binder = None;
            match payload {
                Some(ast::Payload::Record(fs)) => {
                    for f in fs {
                        match &f.pattern {
                            None => {
                                vars.insert(f.field.text.clone(), f.field.text.clone());
                            }
                            Some(ast::Pattern {
                                kind: ast::PatternKind::Bind(x),
                                ..
                            }) => {
                                vars.insert(f.field.text.clone(), x.text.clone());
                            }
                            Some(p) => {
                                fixed.insert(f.field.text.clone(), p.span);
                            }
                        }
                    }
                }
                Some(ast::Payload::Bind(x)) => binder = Some(x.text.clone()),
                _ => {}
            }
            inverse_arms.push(format!("\n    | {product} r -> {} {{ ", path.ctor.text));
            for (k, f) in c_fields.iter().enumerate() {
                if k > 0 {
                    inverse_arms.push(", ");
                }
                if let Some(v) = fixed.get(f) {
                    inverse_arms.push(format!("{f}: "));
                    inverse_arms.append(at.op_text(v.range())?);
                    continue;
                }
                let target = given.iter().find_map(|(g, e)| match &e.kind {
                    ast::ExprKind::Name(q) if vars.get(f) == Some(&q.name.text) => Some(g.clone()),
                    ast::ExprKind::Field(x, ff) if ff.text == *f => match &x.kind {
                        ast::ExprKind::Name(q) if Some(&q.name.text) == binder.as_ref() => {
                            Some(g.clone())
                        }
                        _ => None,
                    },
                    _ => None,
                });
                match target {
                    Some(g) => inverse_arms.push(format!("{f}: r.{g}")),
                    None => return Err(lossy()),
                }
            }
            inverse_arms.push(" }");
        }
        let m = at.module_of(&path.ty);
        let products: Vec<String> = into.iter().map(|d| d.name.text.clone()).collect();
        let mut op = Spelled::from("merge constructor ");
        op.append(type_spelled(at.before(), &m, &path.ty.name.text));
        op.push(format!(".({}) into ", products.join(" | ")));
        op.append(c_text);
        op.push("\n  by match");
        op.append(inverse_arms);
        op.append(at.restored_arms(clauses, &path.ty, std::slice::from_ref(&path.ctor.text))?);
        Ok(vec![at.restructure(
            &path.ty,
            op,
            Undo::Derived,
            Vec::new(),
        )?])
    }

    pub(super) fn merge_inverse(
        &self,
        at: &Inverting,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: Option<&[ast::Arm]>,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let before = at.before();
        let lossy = || {
            vec![Diag::new(
                self.span,
                format!(
                    "the merge into `{}` computes or drops fields, so it has no inverse",
                    into.name.text
                ),
            )]
        };
        let sources: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let module = schema
            .module_named(&at.module_of(ty))
            .unwrap_or(schema::PRELUDE);
        let wanted: BTreeSet<&str> = sources.iter().map(String::as_str).collect();
        let fields = into.record.as_deref().unwrap_or_default();
        let (disc, disc_def) = fields
            .iter()
            .find_map(|f| match &f.ty.kind {
                ast::TypeKind::Named {
                    name,
                    sel: None,
                    args,
                } if args.is_empty() => match schema.lookup_type(module, name) {
                    Ok(schema::TypeName::Decl(d)) => {
                        let st = schema.ty(d);
                        let named: BTreeSet<&str> =
                            st.ctors().iter().map(|c| c.name.as_str()).collect();
                        (st.ctors().iter().all(|c| c.fields.is_empty()) && named == wanted)
                            .then(|| (f.name.text.clone(), d))
                    }
                    _ => None,
                },
                _ => None,
            })
            .ok_or_else(lossy)?;
        let m_fields: Vec<String> = fields
            .iter()
            .map(|f| f.name.text.clone())
            .filter(|f| *f != disc)
            .collect();
        let mut out_arms = Spelled::default();
        let mut products = Spelled::default();
        for (k, src) in sources.iter().enumerate() {
            let (text, src_fields) = at.before_ctor(ty, src)?;
            if k > 0 {
                products.push(" | ");
            }
            products.append(text);
            let mut given = Vec::new();
            for f in &src_fields {
                let g = match arms {
                    None => m_fields.contains(f).then(|| f.clone()),
                    Some(arms) => routed_from(arms, src, f),
                };
                match g {
                    Some(g) => given.push(format!("{f}: {g}")),
                    None => return Err(lossy()),
                }
            }
            let value = schema
                .ty(disc_def)
                .ctor_named(src)
                .and_then(|c| schema.ids().ctor_id(c))
                .map(CatCtorId::raw);
            out_arms.push(format!("\n    | {} {{ {disc}: ", into.name.text));
            out_arms.name(src, Space::Shared, value);
            for f in &m_fields {
                out_arms.push(format!(", {f}"));
            }
            out_arms.push(format!(" }} -> {src} {{ {} }}", given.join(", ")));
        }
        let m = at.module_of(ty);
        let mut op = Spelled::from("split constructor ");
        op.append(type_spelled(before, &m, &ty.name.text));
        op.push(format!(".{} into ", into.name.text));
        op.append(products);
        op.push("\n  by match");
        op.append(out_arms);
        op.append(at.restored_arms(clauses, ty, &sources)?);
        let checks = self.carried_checks_back(at, &schema, ty, &sources)?;
        Ok(vec![at.restructure(ty, op, Undo::Written, checks)?])
    }

    /// Returns each check over some of `sources` of type `ty`, by id, with its text in the source
    /// before the operator, whose schema is `schema`.
    fn carried_checks_back(
        &self,
        at: &Inverting,
        schema: &Schema,
        ty: &QName,
        sources: &[String],
    ) -> R<Vec<(CatConstraintId, Spelled)>> {
        let def = self.type_id(schema, ty)?;
        let td = schema.ty(def);
        let src_ix: Vec<u32> = sources.iter().filter_map(|s| td.ctor_index(s)).collect();
        let rels: Vec<RelId> = Self::relations_of(schema, def)
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        let mut out = Vec::new();
        for (k, chk) in schema.checks.iter().enumerate() {
            let over = rels.contains(&chk.relation)
                && chk
                    .ctors
                    .as_ref()
                    .is_some_and(|cs| cs.iter().any(|c| src_ix.contains(c)));
            if !over {
                continue;
            }
            let Some(id) = schema.ids().check_id(k) else {
                return self.err("the log does not hold the check the operator rewrites");
            };
            out.push((id, at.decl(chk.file.0 as usize)?));
        }
        Ok(out)
    }

    pub(super) fn nest_inverse(
        &self,
        at: &Inverting,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let (before, after) = (at.before(), at.after());
        let m = at.module_of(ty);
        let t = &ty.name.text;
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the constructor",
            )]
        };
        let schema = after
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let td = schema.ty(self.type_id(&schema, ty)?);
        let Some(Ty::Data { def: inner, .. }) = td
            .ctor_named(&into.name.text)
            .and_then(|c| td.ctor_def(c).fields.first())
            .map(|f| &f.ty)
        else {
            return Err(missing());
        };
        let names: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
        let mut arms = at.restored_arms(clauses, ty, &names)?;
        let mut products = Vec::new();
        for c in &names {
            let product = schema
                .ty(*inner)
                .ctor_named(c)
                .and_then(|p| schema.ids().ctor_id(p))
                .ok_or_else(missing)?;
            let was = before.ctor_key_id(&m, t, c).ok_or_else(missing)?;
            arms = arms.aliased(Space::Shared, was, product.raw());
            products.push((product, c.clone()));
        }
        let ty_id = before
            .ids
            .get(&ObjectKey::named(
                KeyKind::Type,
                Owner::Module(m.clone()),
                t,
            ))
            .copied()
            .ok_or_else(missing)?;
        let mut op = Spelled::from("unnest constructor ");
        op.append(type_spelled(before, &m, t));
        op.push(format!(".{}", into.name.text));
        op.append(arms);
        Ok(vec![at.restructure(
            ty,
            op,
            Undo::Unnest(CatTypeId::from_raw(ty_id), products),
            Vec::new(),
        )?])
    }

    pub(super) fn unnest_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let before = at.before();
        let (ctext, _) = at.before_ctor(&path.ty, &path.ctor.text)?;
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let td = schema.ty(self.type_id(&schema, &path.ty)?);
        let products: Vec<String> = td
            .ctor_index(&path.ctor.text)
            .and_then(|c| match &td.ctors()[c as usize].fields.first()?.ty {
                Ty::Data { def: sd, .. } => Some(
                    schema
                        .ty(*sd)
                        .ctors()
                        .iter()
                        .map(|x| x.name.clone())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let m = at.module_of(&path.ty);
        let mut op = Spelled::from("nest constructor ");
        op.append(type_spelled(before, &m, &path.ty.name.text));
        op.push(format!(".({}) into ", products.join(" | ")));
        op.append(ctext);
        op.append(at.restored_arms(clauses, &path.ty, std::slice::from_ref(&path.ctor.text))?);
        Ok(vec![at.restructure(
            &path.ty,
            op,
            Undo::Written,
            Vec::new(),
        )?])
    }

    /// Returns the inverse of `move field path to to`, moving a field of constructor `path.ctor`
    /// up, or with `sel` the header field down to the constructors of `sel`.
    pub(super) fn move_inverse(
        &self,
        at: &Inverting,
        path: &ast::FieldPath,
        sel: Option<&CtorSel>,
    ) -> R<Vec<Inverse>> {
        let (before, after) = (at.before(), at.after());
        let m = at.module_of(&path.ty);
        let t = &path.ty.name.text;
        let f = &path.field.text;
        let ty = type_spelled(before, &m, t);
        let mut op = Spelled::from("move field ");
        op.append(ty.clone());
        op.push(".");
        match (&path.ctor, sel) {
            (Some(c), None) => {
                let mut field = Spelled::default();
                field.name(f, Space::Field, after.field_key_id(&m, t, None, f));
                op.append(field);
                op.push(" to ");
                op.append(ty);
                op.push(".");
                op.append(ctor_spelled(before, &m, t, &c.text));
                Ok(vec![
                    at.restructure(&path.ty, op, Undo::Written, Vec::new())?,
                    self.reorder(before, path, Some(c.text.clone()), at.of())?,
                ])
            }
            (None, Some(sel)) => {
                let names: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                let first = &names[0];
                let header = before.field_key_id(&m, t, None, f);
                let holder = names
                    .iter()
                    .find(|c| after.field_key_id(&m, t, Some(c), f) == header)
                    .unwrap_or(first);
                let mut field = Spelled::default();
                field.name(f, Space::Field, header);
                op.append(ctor_spelled(after, &m, t, holder));
                op.push(".");
                op.append(field);
                op.push(" to ");
                op.append(ty);
                let schema = before
                    .schema()
                    .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
                let def = self.type_id(&schema, &path.ty)?;
                if schema.ty(def).ctors().len() > names.len() {
                    let key = self.named_now(after, &m, t, Some(first), f);
                    let Some(fill) = self.fill_for(at.fills(), &key) else {
                        return self.err(format!(
                            "the inverse of moving `{}` down needs a fill: `fill {key} = e`",
                            path_text(path)
                        ));
                    };
                    op.push(" fill ");
                    op.push(fill);
                }
                let from = names
                    .iter()
                    .map(|c| after.ctor_key_id(&m, t, c).map(CatCtorId::from_raw))
                    .collect::<Option<Vec<CatCtorId>>>()
                    .ok_or_else(|| {
                        vec![Diag::new(
                            self.span,
                            "the log does not hold the constructor",
                        )]
                    })?;
                Ok(vec![
                    at.restructure(&path.ty, op, Undo::UpFrom(from), Vec::new())?,
                    self.reorder(before, path, None, at.of())?,
                ])
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(super) fn added_ctor_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
    ) -> R<Vec<Inverse>> {
        let (before, after) = (at.before(), at.after());
        let m = at.module_of(&path.ty);
        let t = &path.ty.name.text;
        let record = match before.find("type", &m, t).and_then(|i| before.parsed(i)) {
            Some(Decl::Type(
                td @ ast::TypeDecl {
                    body: ast::TypeBody::Record(_),
                    ..
                },
            )) => Some(
                decl_fields(&td)
                    .into_iter()
                    .filter_map(|(c, f)| before.field_key_id(&m, t, c, &f.name.text))
                    .map(CatFieldId::from_raw)
                    .collect(),
            ),
            _ => None,
        };
        let Some(ctor) = after.ctor_key_id(&m, t, &path.ctor.text) else {
            return self.err("the log does not hold the object the operator adds");
        };
        Ok(vec![Inverse::Unadd {
            ctor: CatCtorId::from_raw(ctor),
            record,
            of: at.of(),
        }])
    }

    pub(super) fn removed_ctor_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
    ) -> R<Vec<Inverse>> {
        let before = at.before();
        let m = at.module_of(&path.ty);
        let t = &path.ty.name.text;
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the removed constructor",
            )]
        };
        let i = before.find("type", &m, t).ok_or_else(missing)?;
        let Some(Decl::Type(td)) = before.parsed(i) else {
            return Err(missing());
        };
        let ctors = ctor_extents(&before.decls[i].text, header(&m).len(), &td);
        let k = ctors
            .iter()
            .position(|c| c.0 == path.ctor.text)
            .ok_or_else(missing)?;
        let text = at.decl_range(i, ctors[k].1.start..ctors[k].2)?;
        let after = match k {
            0 => None,
            _ => Some(CatCtorId::from_raw(
                before
                    .ctor_key_id(&m, t, &ctors[k - 1].0)
                    .ok_or_else(missing)?,
            )),
        };
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let def = self.type_id(&schema, &path.ty)?;
        let c = schema
            .ty(def)
            .ctor_index(&path.ctor.text)
            .ok_or_else(missing)?;
        let mut joins = Vec::new();
        for (rel, ..) in Self::relations_of(&schema, def) {
            for s in &schema.relation(rel).sets {
                if s.ctors.contains(&c) {
                    joins.extend(schema.ids().set_id(rel, &s.name));
                }
            }
        }
        let arms = self.removed_arms(at, &schema, def, c)?;
        let ty = before
            .ids
            .get(&ObjectKey::named(
                KeyKind::Type,
                Owner::Module(m.clone()),
                t,
            ))
            .copied()
            .ok_or_else(missing)?;
        let id = before
            .ctor_key_id(&m, t, &path.ctor.text)
            .ok_or_else(missing)?;
        let mut out = vec![Inverse::Readd {
            ty: CatTypeId::from_raw(ty),
            ctor: CatCtorId::from_raw(id),
            name: path.ctor.text.clone(),
            text,
            after,
            joins,
            arms,
            of: at.of(),
        }];
        let (drops, adds) = at.clauses_back(&m, t)?;
        if !drops.is_empty() || !adds.is_empty() {
            out.push(Inverse::Restructure {
                drops,
                op: Spelled::default(),
                adds,
                how: Undo::Written,
                checks: Vec::new(),
                of: at.of(),
            });
        }
        out.push(at.states_back(|s| match s {
            StateOf::Relation(_, c) | StateOf::Value(c) => c.raw() == id,
        }));
        Ok(out)
    }

    /// Returns, for each match over type `def` of each constraint of the source before the
    /// operator that holds arms over constructor `c` of `def`, the constraint's id, the position
    /// of the match among those over `def` in it, the constructor of the arm before the first
    /// over `c`, and its arms over `c`.
    fn removed_arms(
        &self,
        at: &Inverting,
        schema: &Schema,
        def: TypeId,
        c: u32,
    ) -> R<Vec<PlacedArms>> {
        let before = at.before();
        let refs = check::schema_refs(schema);
        let id_of = |k: u32| {
            let r = schema.ty(def).ctor_refs().nth(k as usize)?;
            schema.ids().ctor_id(r)
        };
        let mut out = Vec::new();
        for (k, chk) in schema.constraints.iter().enumerate() {
            let j = chk.file.0 as usize;
            let Some(d @ Decl::Constraint(_)) = before.parsed(j) else {
                continue;
            };
            let jh = header(&before.decls[j].module).len();
            let matches = refs.iter().filter(|(f, _, r)| {
                f.0 as usize == j && matches!(r, Ref::Match(x) if over(schema, x, def))
            });
            for (n, (_, span, _)) in matches.enumerate() {
                let mut texts = Vec::new();
                let mut prev = None;
                let mut beside = None;
                for a in match_arms(&d, *span).unwrap_or_default() {
                    let named = arm_ctor(&refs, j, def, &a);
                    if named == Some(c) {
                        if texts.is_empty() {
                            beside = prev.and_then(id_of);
                        }
                        let start = a.patterns[0].span.start as usize - jh;
                        let end = a.body.span.end as usize - jh;
                        texts.push(at.decl_range(j, start..end)?);
                    } else if named.is_some() {
                        prev = named;
                    }
                }
                if texts.is_empty() {
                    continue;
                }
                let Some(id) = schema.ids().constraint_id(k) else {
                    return self.err("the log does not hold the constraint the operator rewrites");
                };
                out.push((id, n, beside, texts));
            }
        }
        Ok(out)
    }
}

/// Returns the position among the constructors of type `def` of the constructor the pattern of
/// arm `a` of declaration `j` names, as `refs` resolve it.
fn arm_ctor(refs: &[(FileId, Span, Ref)], j: usize, def: TypeId, a: &ast::Arm) -> Option<u32> {
    let ast::PatternKind::Ctor { path, ctor, .. } = &a.patterns.first()?.kind else {
        return None;
    };
    let name = ctor.as_ref().unwrap_or(&path.name).span;
    refs.iter().find_map(|(g, s, x)| match x {
        Ref::Ctor(d, c) if g.0 as usize == j && *s == name && *d == def => Some(*c),
        _ => None,
    })
}

/// Returns whether a match over a value of type `ty` is over type `def`.
fn over(schema: &Schema, ty: &Ty, def: TypeId) -> bool {
    match ty {
        Ty::Data { def: d, .. } => *d == def,
        Ty::Key { rel, .. } => schema.relation(*rel).elem_def == def,
        _ => false,
    }
}

/// Returns the field of the merged constructor that the routing `arms` gives field `f` of source
/// constructor `src`, when an arm carries it whole.
fn routed_from(arms: &[ast::Arm], src: &str, f: &str) -> Option<String> {
    arms.iter().find_map(|a| {
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
            return None;
        };
        if ctor.as_ref().unwrap_or(&path.name).text != *src {
            return None;
        }
        let binder = match payload {
            Some(ast::Payload::Bind(x)) => Some(x.text.clone()),
            _ => None,
        };
        let bound: BTreeMap<String, String> = match payload {
            Some(ast::Payload::Record(fs)) => fs
                .iter()
                .filter_map(|p| match &p.pattern {
                    None => Some((p.field.text.clone(), p.field.text.clone())),
                    Some(ast::Pattern {
                        kind: ast::PatternKind::Bind(x),
                        ..
                    }) => Some((x.text.clone(), p.field.text.clone())),
                    _ => None,
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        let ast::ExprKind::Ctor {
            record: Some(ast::RecordExpr::Fields(fs, _)),
            ..
        } = &a.body.kind
        else {
            return None;
        };
        fs.iter().find_map(|(g, e)| match &e.kind {
            ast::ExprKind::Field(x, ff) if ff.text == *f => match &x.kind {
                ast::ExprKind::Name(q) if Some(&q.name.text) == binder.as_ref() => {
                    Some(g.text.clone())
                }
                _ => None,
            },
            ast::ExprKind::Name(q) if bound.get(&q.name.text).map(String::as_str) == Some(f) => {
                Some(g.text.clone())
            }
            _ => None,
        })
    })
}

/// Returns the path of the constructor of id `c` in `src`, `None` when `src` holds none.
fn ctor_path(src: &Source, c: CatCtorId) -> Option<ast::CtorPath> {
    let held = |space: Space, id: u64| {
        src.ids
            .iter()
            .find(|(k, x)| **x == id && k.kind().space() == space)
            .map(|(k, _)| k)
    };
    let key = held(Space::Shared, c.raw()).filter(|k| k.kind() == KeyKind::Ctor)?;
    let Owner::Object(space, t) = key.owner() else {
        return None;
    };
    let ty = held(*space, *t)?;
    let Owner::Module(m) = ty.owner() else {
        return None;
    };
    let ident = |text: &str| ast::Ident {
        text: text.to_string(),
        span: Span::new(0, 0),
    };
    Some(ast::CtorPath {
        ty: QName {
            module: Some(ident(m)),
            name: ident(ty.name()?),
        },
        ctor: ident(key.name()?),
    })
}

/// Arms of a constructor an inverse puts back in a match of a constraint: the constraint's id,
/// the match's position among those over the type in it, the constructor of the arm they follow,
/// none for first, and the arms.
pub(super) type PlacedArms = (CatConstraintId, usize, Option<CatCtorId>, Vec<Spelled>);

/// Where a constructor an inverse puts back goes: after the constructor of an id, else first;
/// the sets it joins; and its arms in each match of each constraint.
pub(super) type Placing = (Option<CatCtorId>, Vec<CatSetId>, Vec<PlacedArms>);

impl Cx<'_> {
    pub(super) fn restructure(
        &mut self,
        drops: Vec<CatIndexId>,
        [op, adds]: [Spelled; 2],
        how: Undo,
        checks: Vec<(CatConstraintId, Spelled)>,
        of: &str,
        late: &Late,
    ) -> R<()> {
        let text = self.spell(&op, of, late)?;
        let adds = self.spell(&adds, of, late)?;
        let mut set = Vec::new();
        for (c, t) in checks {
            let decl = DeclOf::Object(KeyKind::Constraint, c.raw());
            if decl.find(self.src).is_none() {
                return self.err(dropped(of, "constraint"));
            }
            set.push((decl, self.spell(&t, of, late)?));
        }
        let now = spellings(self.src);
        let named = |c: &CatCtorId| match now.get(&(Space::Shared, c.raw())) {
            Some(n) => Ok(n.to_string()),
            None => self.err(dropped(of, "constructor")),
        };
        let mut from = Vec::new();
        let mut renames = None;
        match &how {
            Undo::UpFrom(ctors) => {
                for c in ctors {
                    from.push(named(c)?);
                }
            }
            Undo::Unnest(ty, products) => {
                let Some((module, t)) = NamedId::Type(*ty).spelled(&self.schema()?) else {
                    return self.err(dropped(of, "type"));
                };
                let mut moved = Vec::new();
                for (c, was) in products {
                    let n = named(c)?;
                    if n != *was {
                        moved.push((n, was.clone()));
                    }
                }
                let taken: BTreeSet<&str> = now
                    .values()
                    .copied()
                    .chain(moved.iter().map(|(_, w)| w.as_str()))
                    .collect();
                let mut through = Vec::new();
                for (n, _) in &moved {
                    let fresh = (1..)
                        .map(|k| format!("{n}{k}"))
                        .find(|x| !taken.contains(x.as_str()) && !through.contains(x))
                        .expect("an unused name");
                    through.push(fresh);
                }
                let mut text = String::new();
                for ((n, _), x) in moved.iter().zip(&through) {
                    text.push_str(&format!("rename constructor {t}.{n} to {x}\n"));
                }
                for ((_, was), x) in moved.iter().zip(&through) {
                    text.push_str(&format!("rename constructor {t}.{x} to {was}\n"));
                }
                renames = (!text.is_empty()).then_some((module, text));
            }
            Undo::Written | Undo::Derived => {}
        }
        if !drops.is_empty() {
            for id in drops {
                self.drop_index_id(id, of)?;
            }
            self.unmapped_step();
        }
        let how = match how {
            Undo::Written | Undo::Unnest(..) => How::Written,
            Undo::Derived => How::Derived,
            Undo::UpFrom(_) => How::UpFrom(&from),
        };
        if !text.text.is_empty() {
            self.undo_ops(&text, how, &set)?;
        } else if !set.is_empty() {
            for (decl, check) in set {
                if let Some((i, ..)) = decl.find(self.src) {
                    self.src.rewrite(i, check);
                }
            }
            self.src.settle_ids();
            self.unmapped_step();
        }
        if let Some((module, text)) = renames {
            self.undo_in(module, text)?;
        }
        if !adds.text.is_empty() {
            self.undo_ops(&adds, How::Written, &[])?;
        }
        Ok(())
    }

    /// Record the source as a step that moves no rows.
    fn unmapped_step(&self) {
        if let Some(steps) = self.steps {
            steps.borrow_mut().push(Step {
                after: self.src.clone(),
                maps: Vec::new(),
                applied: None,
            });
        }
    }

    /// Drop the unique or index of id `id`, refusing as the inverse of `of` when a later migration
    /// dropped it.
    fn drop_index_id(&mut self, id: CatIndexId, of: &str) -> R<()> {
        let schema = self.schema()?;
        let held = schema.ids().index_of(id).and_then(|(rel, _)| {
            let (i, _, ranges) = relation_clauses(self.src, &schema, rel)?;
            let (.., k, _) = index_clauses(self.src, &schema, rel)
                .into_iter()
                .find(|c| c.0 == id.raw())?;
            Some((i, ranges, k))
        });
        let Some((i, ranges, k)) = held else {
            return self.err(dropped(of, "index"));
        };
        self.src.forget_id(KeyKind::Index.space(), id.raw());
        self.drop_clause(i, &ranges, k);
        self.src.settle_ids();
        Ok(())
    }

    pub(super) fn unadd(
        &mut self,
        ctor: CatCtorId,
        record: Option<Vec<CatFieldId>>,
        of: &str,
    ) -> R<()> {
        let Some(path) = ctor_path(self.src, ctor) else {
            return self.err(dropped(of, "constructor"));
        };
        self.remove_ctor(&path, true)?;
        let Some(order) = record else {
            return Ok(());
        };
        let (i, t) = self.type_decl(&path.ty)?;
        match &t.body {
            ast::TypeBody::Ctors(cs) if cs.len() == 1 && cs[0].name.text == t.name.text => {}
            _ => return Ok(()),
        }
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let head = match field_braces(&text, h, &t, None) {
            Some(b) => b.start,
            None => text.find('=').unwrap_or(text.len()),
        };
        let items: Vec<&str> = decl_fields(&t)
            .into_iter()
            .map(|(_, f)| {
                let start = f.name.span.start as usize - h;
                text[start..item_end(&text, start)].trim_end()
            })
            .collect();
        let record = format!("{} = {{ {} }}", text[..head].trim_end(), items.join(", "));
        self.unsum(&path.ty, record.into())?;
        let now = spellings(self.src);
        let names: Vec<String> = order
            .iter()
            .filter_map(|f| now.get(&(Space::Field, f.raw())))
            .map(ToString::to_string)
            .collect();
        self.reorder_fields(&path.ty, None, &names)
    }

    pub(super) fn readd(
        &mut self,
        (ty, ctor, name): (CatTypeId, CatCtorId, &str),
        text: &Spelled,
        (after, joins, arms): Placing,
        of: &str,
        late: &Late,
    ) -> R<()> {
        let Some(q) = NamedId::Type(ty).qualified(self.src) else {
            return self.err(dropped(of, "type"));
        };
        let now = spellings(self.src);
        let after = match after {
            Some(c) => match now.get(&(Space::Shared, c.raw())) {
                Some(n) => Some(n.to_string()),
                None => return self.err(dropped(of, "constructor")),
            },
            None => None,
        };
        let text = self.spell(text, of, late)?;
        let mut sets = Vec::new();
        for s in joins {
            match DeclOf::Object(KeyKind::Set, s.raw()).find(self.src) {
                Some((k, ..)) => sets.push(k),
                None => return self.err(dropped(of, "set")),
            }
        }
        let schema = self.schema()?;
        let def = self.type_id(&schema, &q)?;
        let refs = check::schema_refs(&schema);
        let mut placed = Vec::new();
        for (c, n, beside, texts) in arms {
            let Some((k, ..)) = DeclOf::Object(KeyKind::Constraint, c.raw()).find(self.src) else {
                return self.err(dropped(of, "constraint"));
            };
            let beside = match beside {
                Some(b) => match schema.ids().ctor_of(b) {
                    Some(r) if r.def() == def => Some(r.index()),
                    _ => return self.err(dropped(of, "constructor")),
                },
                None => None,
            };
            let at = refs
                .iter()
                .filter(|(f, _, r)| {
                    f.0 as usize == k && matches!(r, Ref::Match(x) if over(&schema, x, def))
                })
                .nth(n);
            let (Some((_, at, _)), Some(d)) = (at, self.src.parsed(k)) else {
                continue;
            };
            let h = header(&self.src.decls[k].module).len();
            let arms = match_arms(&d, *at).unwrap_or_default();
            let place = match beside {
                Some(b) => arms
                    .iter()
                    .find(|a| arm_ctor(&refs, k, def, a) == Some(b))
                    .or(arms.last())
                    .map(|a| (a.body.span.end as usize - h, false)),
                None => arms
                    .first()
                    .map(|a| (a.patterns[0].span.start as usize - h, true)),
            };
            if let Some((end, first)) = place {
                placed.push((k, end, first, texts));
            }
        }
        let (i, t) = self.type_decl(&q)?;
        let h = header(&self.src.decls[i].module).len();
        let ctors = ctor_extents(&self.src.decls[i].text, h, &t);
        let edit = match after.and_then(|a| ctors.iter().find(|c| c.0 == a)) {
            Some(c) => (c.2..c.2, Written::concat(["\n  | ".into(), text])),
            None => match ctors.first() {
                Some(c) => (
                    c.1.start..c.1.start,
                    Written::concat([text, "\n  | ".into()]),
                ),
                None => return self.err("the type has no constructors to place it beside"),
            },
        };
        self.src.edit(i, vec![edit]);
        for k in sets {
            let Some(Decl::Set(sd)) = self.src.parsed(k) else {
                continue;
            };
            let at = sd.span.end as usize - header(&self.src.decls[k].module).len();
            self.src.edit(k, vec![(at..at, format!(" | {name}"))]);
        }
        self.src.settle_ids();
        let (m, tn) = (self.module_of(&q), q.name.text.clone());
        let again: Recreated = self
            .src
            .ctor_key_id(&m, &tn, name)
            .map(|id| ((Space::Shared, ctor.raw()), id))
            .into_iter()
            .collect();
        let mut edits: BTreeMap<usize, Vec<(Range<usize>, Written)>> = BTreeMap::new();
        for (k, at, first, texts) in placed {
            let mut text = Vec::new();
            for t in texts {
                let t = self.spell(&t.mapped(&again), of, late)?;
                if first {
                    text.extend([t, "\n  | ".into()]);
                } else {
                    text.extend(["\n  | ".into(), t]);
                }
            }
            edits
                .entry(k)
                .or_default()
                .push((at..at, Written::concat(text)));
        }
        for (k, es) in edits {
            self.src.edit(k, es);
        }
        Ok(())
    }
}

impl Inverse {
    /// Returns the structural step with each id an earlier step of the revert created again
    /// replaced by the id `again` maps it to.
    pub(super) fn mapped_structural(self, again: &Recreated) -> Inverse {
        let shared = |id: u64| again_id(again, Space::Shared, id);
        let ctor = |c: CatCtorId| CatCtorId::from_raw(shared(c.raw()));
        let constraint = |c: CatConstraintId| {
            CatConstraintId::from_raw(again_id(again, Space::Constraint, c.raw()))
        };
        match self {
            Inverse::Restructure {
                drops,
                op,
                adds,
                how,
                checks,
                of,
            } => Inverse::Restructure {
                drops: drops
                    .into_iter()
                    .map(|i| CatIndexId::from_raw(again_id(again, KeyKind::Index.space(), i.raw())))
                    .collect(),
                op: op.mapped(again),
                adds: adds.mapped(again),
                how: match how {
                    Undo::UpFrom(cs) => Undo::UpFrom(cs.into_iter().map(ctor).collect()),
                    Undo::Unnest(ty, products) => Undo::Unnest(
                        CatTypeId::from_raw(shared(ty.raw())),
                        products.into_iter().map(|(c, n)| (ctor(c), n)).collect(),
                    ),
                    how => how,
                },
                checks: checks
                    .into_iter()
                    .map(|(c, t)| (constraint(c), t.mapped(again)))
                    .collect(),
                of,
            },
            Inverse::Unadd {
                ctor: c,
                record,
                of,
            } => Inverse::Unadd {
                ctor: ctor(c),
                record: record.map(|fs| {
                    fs.into_iter()
                        .map(|f| CatFieldId::from_raw(again_id(again, Space::Field, f.raw())))
                        .collect()
                }),
                of,
            },
            Inverse::Readd {
                ty,
                ctor: c,
                name,
                text,
                after,
                joins,
                arms,
                of,
            } => Inverse::Readd {
                ty: CatTypeId::from_raw(shared(ty.raw())),
                ctor: ctor(c),
                name,
                text: text.mapped(again),
                after: after.map(ctor),
                joins: joins
                    .into_iter()
                    .map(|s| CatSetId::from_raw(again_id(again, Space::Set, s.raw())))
                    .collect(),
                arms: arms
                    .into_iter()
                    .map(|(c, n, b, ts)| {
                        (
                            constraint(c),
                            n,
                            b.map(ctor),
                            ts.into_iter().map(|t| t.mapped(again)).collect(),
                        )
                    })
                    .collect(),
                of,
            },
            _ => unreachable!("mapped_structural is given a structural step"),
        }
    }
}
