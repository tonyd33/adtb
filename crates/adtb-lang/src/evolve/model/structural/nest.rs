use super::split::kind_word;
use super::{ctor_in, def_in, sel_idents};
use crate::ast::{self, CtorSel, QName};
use crate::evolve::carry::{self, Structural};
use crate::evolve::ctor::element_head;
use crate::evolve::model::field::Applied;
use crate::evolve::model::op::{Res, Resolving, broken, refuse, refuse_at, unmodelled};
use crate::evolve::model::reselect::Mode;
use crate::evolve::model::run::{Built, Run};
use crate::evolve::model::*;
use crate::evolve::{Clause, Resolved, RowMap};
use crate::schema::{Schema, TypeDefKind};
use crate::span::FileId;
use crate::types::{Ty, TypeId};

/// Returns whether the field of nesting constructor `nc` of type `def` in `after` holds a sum
/// without a header whose constructors are `sources` of `def` in `before`, by name and fields.
fn holds_sources(
    before: &Schema,
    after: &Schema,
    def: TypeId,
    sources: &[u32],
    inner: &Ty,
) -> bool {
    let Ty::Data {
        def: s,
        args,
        ctors: None,
    } = inner
    else {
        return false;
    };
    if !args.is_empty() {
        return false;
    }
    let td = before.ty(def);
    let st = after.ty(*s);
    st.header().is_empty()
        && st.ctors().len() == sources.len()
        && sources.iter().all(|&k| {
            let src = &td.ctors()[k as usize];
            st.ctor_index(&src.name).is_some_and(|j| {
                let b = &st.ctors()[j as usize].fields;
                src.fields.len() == b.len()
                    && src
                        .fields
                        .iter()
                        .all(|f| b.iter().any(|g| g.name == f.name && g.ty.alike(&f.ty)))
            })
        })
}

impl Model {
    /// Returns the names of the header fields of type `t`.
    pub(super) fn header_names(&self, t: u64) -> Vec<String> {
        self.types[&t]
            .header
            .iter()
            .flatten()
            .map(|f| self.fields[f].name.clone())
            .collect()
    }

    /// Returns whether constructor `c` has a field named `name`.
    fn has_field(&self, c: u64, name: &str) -> bool {
        self.ctors[&c]
            .fields
            .iter()
            .flatten()
            .any(|f| self.fields[f].name == name)
    }

    /// Set clause `at` of relation variable `r` to elements `elems` and scope `parts`, a new
    /// clause when `rescoped`.
    fn rewrite_clause(
        &mut self,
        (r, at, rescoped): (u64, usize, bool),
        elems: Vec<Term>,
        parts: Vec<Term>,
    ) {
        let rel = self.relations.get_mut(&r).expect("a relation variable");
        if let ClauseM::Index {
            id,
            elems: e,
            scope,
            ..
        } = &mut rel.clauses[at]
        {
            if rescoped {
                *id = None;
            }
            *e = elems;
            *scope = Some(Scope {
                listed: parts.len() > 1,
                parts,
            });
        }
    }

    /// Build after a structural operator, and lift the type `t` and each relation variable of
    /// `rels` again from the build. Returns `None` when the declarations do not build, the
    /// operator's arms then left out.
    pub(super) fn rebuilt(&mut self, run: &Run, t: u64, rels: &[u64]) -> Res<Option<Built>> {
        let Ok(built) = self.build_adopting(&[], &run.ahead(self)) else {
            return Ok(None);
        };
        self.relift(&built, Item::Type(t))?;
        for &r in rels {
            self.relift(&built, Item::Relation(r))?;
        }
        let mut probe = self.clone();
        let respelled: Vec<Item> = (0..self.decls.len())
            .filter(|&i| probe.decl_terms_mut(i).iter().any(|x| x.dropped()))
            .map(|i| self.decls[i].item)
            .collect();
        for item in respelled {
            self.relift(&built, item)?;
        }
        match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => Ok(Some(b)),
            Err(_) => broken("declarations lifted again from a build do not build"),
        }
    }

    pub(super) fn nest(
        &mut self,
        run: &Run,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: &[ast::ArmClause],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let t = Resolving::new(self, run.module()).ty(ty)?;
        let sources = self.sources(t, sel)?;
        self.fresh_ctor(t, into)?;
        let n = into.name.text.clone();
        let fields = into.record.as_deref().unwrap_or_default();
        let [field] = fields else {
            return refuse_at(
                into.name.span,
                format!("the nesting constructor `{n}` has exactly one field"),
            );
        };
        let def = def_in(schema, t);
        let src_ix: Vec<u32> = sources.iter().map(|&c| ctor_in(schema, c)).collect();
        let mut probe = self.clone();
        let nc = probe.new_ctor(run, t, &n, Some(fields));
        if let TypeBody::Ctors(cs) = &mut probe.types.get_mut(&t).expect("the type").body {
            cs.push(nc);
        }
        let probed = match probe.built(&[], &run.ahead(&probe)) {
            Ok((b, _)) => b,
            Err(ds) => return refuse(ds[0].diag.message.clone()),
        };
        let ps = probed.schema();
        let inner = &ps.ty(def_in(ps, t)).ctors()[ctor_in(ps, nc) as usize].fields[0].ty;
        if !holds_sources(schema, ps, def, &src_ix, inner) {
            let names: Vec<&str> = sources
                .iter()
                .map(|c| self.ctors[c].name.as_str())
                .collect();
            return refuse_at(
                field.ty.span,
                format!(
                    "the field of `{n}` holds an existing sum whose constructors have the names and fields of {}",
                    names.join(", ")
                ),
            );
        }
        let stages = self.restated(&built, t, &sources, true)?;
        let c = self.new_ctor(run, t, &n, Some(fields));
        let plan = self.carry_plan(
            run,
            &built,
            Structural::Nest,
            t,
            (&sources, std::slice::from_ref(&n)),
        );
        let mut edits = Vec::new();
        let f = field.name.text.clone();
        let rewrites = self.nest_clauses(&built, t, (&sources, c), &f)?;
        edits.extend(self.arm_edits(run, &built, t, &sources, arms)?);
        edits.extend(self.reselect(
            &built,
            t,
            &sources,
            &[c],
            Mode::structural("the nest", true, true),
        )?);
        let mut given = carry::Given::new();
        for &s in &sources {
            let names = self.ctors[&s]
                .fields
                .iter()
                .flatten()
                .map(|x| Some(self.fields[x].name.clone()))
                .collect();
            given.insert((self.ctors[&s].name.clone(), n.clone(), f.clone()), names);
        }
        let reaches: carry::Reaches = sources
            .iter()
            .map(|s| (self.ctors[s].name.clone(), n.clone()))
            .collect();
        let maps = vec![RowMap::Nest {
            def,
            from: src_ix,
            into: n.clone(),
        }];
        let members: Vec<Clause> = sel_idents(sel)
            .iter()
            .filter_map(|x| self.ctor_named(t, &x.text))
            .map(|ctor| Clause::Member { ctor })
            .collect();
        let before = self.clone();
        self.edit_decls(edits)?;
        let rels: Vec<u64> = rewrites.iter().map(|r| r.0.0).collect();
        for (at, fs, parts) in rewrites {
            self.rewrite_clause(at, fs, parts);
        }
        self.replace_ctors(t, &sources, &[c]);
        self.rebind(&before);
        self.restate(&[c], &stages);
        let file = FileId(self.decl_at(Item::Type(t)).expect("the type") as u32);
        let mut clauses = members;
        let decl = match self.rebuilt(run, t, &rels)? {
            Some(after) => {
                if !arms.is_empty() {
                    clauses.extend(self.insert_arms(run, &after, t, &[c], arms)?);
                }
                self.ctor_canonical(&after, run, file, &n, true)?
            }
            None => String::new(),
        };
        self.settle_ids(run);
        self.carry_grants(t, plan, (&reaches, &given), &[], &[]);
        Ok((Resolved::Nest { ctor: c, decl }, clauses, Ok(maps)))
    }

    pub(super) fn unnest(
        &mut self,
        run: &Run,
        path: &ast::CtorPath,
        arms: &[ast::ArmClause],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let t = Resolving::new(self, run.module()).ty(&path.ty)?;
        let tname = self.types[&t].name.clone();
        let n = path.ctor.text.clone();
        let Some(c) = self.ctor_named(t, &n) else {
            return refuse(format!(
                "`{n}` is not a constructor of `{}`",
                path.ty.name.text
            ));
        };
        let def = def_in(schema, t);
        let ci = ctor_in(schema, c);
        let td = schema.ty(def);
        let [field] = td.ctors()[ci as usize].fields.as_slice() else {
            return refuse_at(
                path.ctor.span,
                format!("`{n}` holds one field of a sum, which unnest lifts"),
            );
        };
        let Ty::Data { def: s, args, .. } = &field.ty else {
            return refuse_at(
                path.ctor.span,
                format!("the field of `{n}` holds no declared sum"),
            );
        };
        let st = schema.ty(*s);
        if !args.is_empty()
            || !st.header().is_empty()
            || !matches!(st.kind, TypeDefKind::Sum { record: false, .. })
        {
            return refuse_at(
                path.ctor.span,
                format!("the field of `{n}` holds no declared sum without a header and parameters"),
            );
        }
        let products: Vec<String> = st.ctors().iter().map(|x| x.name.clone()).collect();
        for p in &products {
            if *p != n && self.ctor_named(t, p).is_some() {
                return refuse(format!("`{p}` is already a constructor of `{tname}`"));
            }
        }
        let Some(inner) = schema.ids().type_id(*s).map(u64::from) else {
            return broken("a sum the built schema gives no id");
        };
        let Some(inner_ctors) = self.types.get(&inner).map(|_| self.ctors_of(inner)) else {
            return unmodelled("an unnest of a sum of no user module");
        };
        let f = field.name.clone();
        let mut made = Vec::new();
        for &ic in &inner_ctors {
            let p = self.fresh(Space::Shared);
            let own = self.ctors[&ic].fields.clone().map(|fs| {
                fs.iter()
                    .map(|x| {
                        let id = self.fresh(Space::Field);
                        let mut held = self.fields[x].clone();
                        held.owner = p;
                        self.fields.insert(id, held);
                        id
                    })
                    .collect()
            });
            self.ctors.insert(
                p,
                CtorM {
                    ty: t,
                    name: self.ctors[&ic].name.clone(),
                    fields: own,
                },
            );
            made.push(p);
        }
        let plan = self.carry_plan(run, &built, Structural::Unnest, t, (&[c], &products));
        let mut edits = Vec::new();
        let rewrites = self.unnest_clauses(&built, t, (c, &made), &products, &f)?;
        edits.extend(self.arm_edits(run, &built, t, &[c], arms)?);
        edits.extend(self.reselect(
            &built,
            t,
            &[c],
            &made,
            Mode::structural("the unnest", false, true),
        )?);
        let stages = self.restated(&built, t, &[c], false)?;
        let mut given = carry::Given::new();
        for x in st.ctors() {
            for g in &x.fields {
                given.insert(
                    (n.clone(), x.name.clone(), g.name.clone()),
                    vec![Some(f.clone())],
                );
            }
        }
        let reaches: carry::Reaches = products.iter().map(|p| (n.clone(), p.clone())).collect();
        let before = self.clone();
        self.edit_decls(edits)?;
        let rels: Vec<u64> = rewrites.iter().map(|r| r.0.0).collect();
        for (at, fs, parts) in rewrites {
            self.rewrite_clause(at, fs, parts);
        }
        self.replace_ctors(t, &[c], &made);
        self.rebind(&before);
        self.restate(&made, &stages);
        let mut clauses = Vec::new();
        if let Some(after) = self.rebuilt(run, t, &rels)?
            && !arms.is_empty()
        {
            clauses.extend(self.insert_arms(run, &after, t, &made, arms)?);
        }
        self.settle_ids(run);
        self.carry_grants(t, plan, (&reaches, &given), &[], std::slice::from_ref(&f));
        Ok((
            Resolved::Unnest { ctor: c },
            clauses,
            Ok(vec![RowMap::Unnest { def, from: ci }]),
        ))
    }

    /// Returns how a nest of constructors `sources` of type `t` into `c`, whose one field is `f`,
    /// rewrites each unique and index clause over them, as `built` resolves the declarations
    /// before it.
    fn nest_clauses(
        &self,
        built: &Built,
        t: u64,
        (sources, c): (&[u64], u64),
        f: &str,
    ) -> Res<Vec<ClauseRewrite>> {
        let header = self.header_names(t);
        let mut rewrites = Vec::new();
        for cl in self.scoped(built, t, sources, &[c]) {
            let Some(reached) = &cl.reached else { continue };
            let inside: Vec<u64> = reached
                .iter()
                .copied()
                .filter(|x| sources.contains(x))
                .collect();
            if inside.is_empty() {
                continue;
            }
            let shown = format!("{} ({})", kind_word(cl.kind), cl.elems.join(", "));
            let mut fs = Vec::new();
            let mut same = true;
            let held = &self.relations[&cl.relation].clauses[cl.at];
            let ClauseM::Index { elems, scope, .. } = held else {
                return broken("a scoped clause that is no index");
            };
            for (g, e) in cl.elems.iter().zip(elems) {
                let head = element_head(g).0;
                if header.iter().any(|h| h == head) {
                    fs.push(e.clone());
                } else if let ([one], 1) = (inside.as_slice(), reached.len())
                    && self.has_field(*one, head)
                {
                    same = false;
                    fs.push(Term::text(&format!("{f}.{}.{g}", self.ctors[one].name)));
                } else {
                    return refuse(format!("`{shown}` has no image under the nest"));
                }
            }
            let parts = match &cl.rescoped {
                None if same => continue,
                None => scope.as_ref().expect("a scoped clause").parts.clone(),
                Some(parts) => {
                    if reached.len() != inside.len() && inside.len() != sources.len() {
                        return refuse(format!(
                            "`{shown}` ranges over some but not all of the nested constructors and has no image under the nest"
                        ));
                    }
                    parts.clone()
                }
            };
            rewrites.push(((cl.relation, cl.at, cl.rescoped.is_some()), fs, parts));
        }
        Ok(rewrites)
    }

    /// Returns how an unnest of constructor `c` of type `t` into `made`, named `products`, rewrites
    /// each unique and index clause over `c`, whose one field is `field`, as `built` resolves
    /// the declarations before it: each clause's place, elements and scope.
    fn unnest_clauses(
        &self,
        built: &Built,
        t: u64,
        (c, made): (u64, &[u64]),
        products: &[String],
        field: &str,
    ) -> Res<Vec<ClauseRewrite>> {
        let mut rewrites = Vec::new();
        for cl in self.scoped(built, t, &[c], made) {
            if cl.reached.as_ref().is_some_and(|r| !r.contains(&c)) {
                continue;
            }
            let shown = format!("{} ({})", kind_word(cl.kind), cl.elems.join(", "));
            let held = &self.relations[&cl.relation].clauses[cl.at];
            let ClauseM::Index { elems, .. } = held else {
                return broken("a scoped clause that is no index");
            };
            let mut target: Option<String> = None;
            let mut fs = Vec::new();
            for (g, e) in cl.elems.iter().zip(elems) {
                if self.header_names(t).iter().any(|h| h == element_head(g).0) {
                    fs.push(e.clone());
                    continue;
                }
                let parts: Vec<&str> = g.split('.').map(str::trim).collect();
                match parts.as_slice() {
                    [h, ctor, leaf @ ..]
                        if *h == field
                            && !leaf.is_empty()
                            && products.iter().any(|p| p == ctor) =>
                    {
                        if target.as_ref().is_some_and(|x| x != ctor) {
                            return refuse(format!("`{shown}` has no image under the unnest"));
                        }
                        target = Some(ctor.to_string());
                        fs.push(Term::text(&leaf.join(".")));
                    }
                    _ => return refuse(format!("`{shown}` has no image under the unnest")),
                }
            }
            let parts = match (&target, &cl.reached) {
                (Some(p), reached) if reached.as_ref().is_none_or(|r| r.len() == 1) => {
                    let k = products.iter().position(|x| x == p).expect("a product");
                    vec![Term::name((Space::Shared, made[k]), p)]
                }
                (Some(_), _) => {
                    return refuse(format!("`{shown}` has no image under the unnest"));
                }
                (None, None) => continue,
                (None, Some(_)) => match &cl.rescoped {
                    None => continue,
                    Some(parts) => parts.clone(),
                },
            };
            rewrites.push(((cl.relation, cl.at, true), fs, parts));
        }
        Ok(rewrites)
    }
}

/// A rewrite of a unique or index clause: its relation variable, position and whether it
/// becomes a new clause, with its elements and scope.
type ClauseRewrite = ((u64, usize, bool), Vec<Term>, Vec<Term>);
