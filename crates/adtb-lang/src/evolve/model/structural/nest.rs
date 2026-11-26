use super::split::kind_word;
use super::{ctor_in, def_in, sel_idents};
use crate::ast::{self, CtorSel};
use crate::evolve::carry::{self, Structural};
use crate::evolve::ctor::element_head;
use crate::evolve::model::defaults::Taken;
use crate::evolve::model::field::Applied;
use crate::evolve::model::indexes::Elem;
use crate::evolve::model::op::{Fail, Res, broken, refuse, refuse_at};
use crate::evolve::model::reselect::{Edit, Mode, sel_range};
use crate::evolve::model::run::{Built, Late, Run};
use crate::evolve::model::*;
use crate::evolve::{Clause, Resolved, RowMap};
use crate::ids::CatCtorId;
use crate::schema::{Schema, TypeDefKind};
use crate::span::{FileDiag, FileId};
use crate::types::{Ty, TypeId};

/// Returns each of `sources` of `def` in `before` with the id of the constructor of `inner`, a
/// sum of `after`, named as it.
fn nested_tags(
    before: &Schema,
    after: &Schema,
    def: TypeId,
    sources: &[u32],
    inner: &Ty,
) -> Res<Vec<(u32, CatCtorId)>> {
    let Ty::Data { def: s, .. } = inner else {
        return broken("a nesting field not typed by a sum");
    };
    let sum = after.ty(*s);
    sources
        .iter()
        .map(|&k| {
            let name = &before.ty(def).ctors()[k as usize].name;
            let id = sum
                .ctor_index(name)
                .and_then(|j| sum.ctor_at(j))
                .and_then(|c| after.ids().ctor_id(c))
                .ok_or_else(|| Fail::Broken("a nested constructor without an id".into()))?;
            Ok((k, id))
        })
        .collect()
}

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
    pub(super) fn has_field(&self, c: u64, name: &str) -> bool {
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
        if rescoped {
            rel.reimage(at);
        }
        rel.carry_elems(at, elems);
        rel.scope_to(at, parts);
    }

    /// Build after a structural operator, and lift the type `t` and each relation variable of
    /// `rels` again from the build. Returns the failures when the declarations do not build.
    pub(super) fn rebuilt(
        &mut self,
        run: &mut Run,
        t: u64,
        rels: &[u64],
    ) -> Res<Result<Built, Vec<FileDiag>>> {
        self.restate_held()?;
        let built = match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => return Ok(Err(ds)),
        };
        self.relift(&built, Item::Type(t))?;
        for &r in rels {
            self.relift(&built, Item::Relation(r))?;
        }
        let mut probe = self.clone();
        let respelled: Vec<Item> = (0..self.decls.len())
            .filter(|&i| {
                let defaults = match self.decls[i].item {
                    Item::Relation(r) => self.relations[&r].defaults.values().any(Term::dropped),
                    _ => false,
                };
                defaults || probe.decl_terms_mut(i).iter().any(|x| x.dropped())
            })
            .map(|i| self.decls[i].item)
            .collect();
        for item in respelled {
            self.relift(&built, item)?;
        }
        match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => Ok(Ok(b)),
            Err(_) => broken("declarations lifted again from a build do not build"),
        }
    }

    pub(in crate::evolve::model) fn nest(
        &mut self,
        run: &mut Run,
        t: u64,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: &[ast::ArmClause],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
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
        let tags = nested_tags(schema, ps, def, &src_ix, inner)?;
        self.paths_image(t, &sources, "the nest")?;
        let stages = self.restated(&built, t, &sources, true)?;
        let c = self.new_ctor(run, t, &n, Some(fields));
        self.defaults_image(&built, t, (&sources, &[c]), "the nest", Taken::Nest)?;
        let plan = self.carry_plan(
            run,
            &built,
            Structural::Nest,
            t,
            (&sources, std::slice::from_ref(&n)),
        );
        let mut edits = Vec::new();
        let f = field.name.text.clone();
        edits.extend(self.nested_checks(&built, t, &sources, (&n, &f))?);
        let rewrites = self.nest_clauses(&built, t, (&sources, c), &f)?;
        let (arm_edits, written) = self.arm_writes(run, &built, t, (&sources, &sources), arms)?;
        edits.extend(arm_edits);
        edits.extend(self.reselect_rows(&built, t, (&sources, &[c]), "the nest", true)?);
        edits.extend(self.reselect(
            &built,
            t,
            &sources,
            &[c],
            Mode::structural("the nest", true, false),
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
            into: cat_ctor(c),
            from: tags,
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
        let after = self.rebuilt(run, t, &rels)?;
        clauses.extend(self.arm_clauses(run, after.as_ref().map_err(Vec::as_slice), written)?);
        let after = after.ok();
        let decl = match &after {
            Some(after) => self.ctor_canonical(after, run, file, c, true)?,
            None => run.late(Late::Ctor {
                ctor: c,
                record: true,
            }),
        };
        self.settle_ids(run);
        self.carry_grants(t, plan, (&reaches, &given), &[], &[]);
        Ok((Resolved::Nest { ctor: c, decl }, clauses, Ok(maps)))
    }

    pub(in crate::evolve::model) fn unnest(
        &mut self,
        run: &mut Run,
        (t, c): (u64, u64),
        path: &ast::CtorPath,
        arms: &[ast::ArmClause],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let tname = self.types[&t].name.clone();
        let n = self.ctors[&c].name.clone();
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
        let not_declared = || refuse(format!("`{}` is not a type of the schema", st.name));
        let Some(inner) = schema.ids().type_id(*s).map(u64::from) else {
            return not_declared();
        };
        let Some(inner_ctors) = self.types.get(&inner).map(|_| self.ctors_of(inner)) else {
            return not_declared();
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
        let unnested: Vec<(CatCtorId, CatCtorId)> = inner_ctors
            .iter()
            .zip(&made)
            .map(|(&ic, &p)| (cat_ctor(ic), cat_ctor(p)))
            .collect();
        self.paths_image(t, &[c], "the unnest")?;
        self.defaults_image(&built, t, (&[c], &made), "the unnest", Taken::Unnest)?;
        let plan = self.carry_plan(run, &built, Structural::Unnest, t, (&[c], &products));
        let mut edits = Vec::new();
        let rewrites = self.unnest_clauses(&built, t, (c, &made), &products, &f)?;
        let (arm_edits, written) = self.arm_writes(run, &built, t, (&[c], &[c]), arms)?;
        edits.extend(arm_edits);
        edits.extend(self.reselect_rows(&built, t, (&[c], &made), "the unnest", false)?);
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
        let after = self.rebuilt(run, t, &rels)?;
        let clauses = self.arm_clauses(run, after.as_ref().map_err(Vec::as_slice), written)?;
        self.settle_ids(run);
        self.carry_grants(t, plan, (&reaches, &given), &[], std::slice::from_ref(&f));
        Ok((
            Resolved::Unnest { ctor: c },
            clauses,
            Ok(vec![RowMap::Unnest {
                def,
                from: ci,
                products: unnested,
            }]),
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
            let Some(held) = self.relations[&cl.relation].index_at(cl.at) else {
                return broken("a scoped clause that is no index");
            };
            for (g, e) in cl.elems.iter().zip(held.elems().iter().map(Elem::term)) {
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
                None => held.scope_parts().expect("a scoped clause"),
                Some(parts) => {
                    if same && inside.len() != sources.len() {
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

    /// Returns the edits that carry each check over the nested `sources` of type `t` to
    /// constructor `n`, whose one field is `f`, as `built` resolves the declarations: its
    /// predicate under a match on `f` that binds each nested constructor's fields by name, with
    /// `true` for those it does not range over. Refuse a check that also ranges over a
    /// constructor the nest keeps.
    fn nested_checks(
        &self,
        built: &Built,
        t: u64,
        sources: &[u64],
        (n, f): (&str, &str),
    ) -> Res<Vec<Edit>> {
        let schema = built.schema();
        let def = def_in(schema, t);
        let src_ix: Vec<u32> = sources.iter().map(|&c| ctor_in(schema, c)).collect();
        let rels: Vec<crate::types::RelId> = crate::evolve::Cx::relations_of(schema, def)
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        let mut edits = Vec::new();
        for chk in &schema.checks {
            let Some(cs) = &chk.ctors else { continue };
            if !rels.contains(&chk.relation) {
                continue;
            }
            let inside = cs.iter().filter(|c| src_ix.contains(c)).count();
            if inside == 0 {
                continue;
            }
            if inside < cs.len() {
                return refuse(format!(
                    "check `{}` has no image under the nest: it ranges over a constructor the nest keeps",
                    chk.name
                ));
            }
            let j = chk.file.0 as usize;
            let Some(ast::Decl::Check(cd)) = built.decl(chk.file) else {
                continue;
            };
            let h = format!("module {}\n", self.decls[j].module).len();
            if let Some(sel) = &cd.on.sel {
                let idents: Vec<ast::Ident> = sel_idents(sel).into_iter().cloned().collect();
                let r = sel_range(built.text(chk.file), &idents);
                edits.push(Edit::new(j, r.start - h..r.end - h, Term::text(n)));
            }
            let p = cd.predicate.span;
            let pred = p.start as usize - h..p.end as usize - h;
            let body = &built.text(chk.file)[h..][pred.clone()];
            let arms: Vec<String> = sources
                .iter()
                .zip(&src_ix)
                .map(|(s, c)| {
                    let ctor = &self.ctors[s];
                    let fields: Vec<&str> = ctor
                        .fields
                        .iter()
                        .flatten()
                        .map(|x| self.fields[x].name.as_str())
                        .collect();
                    let pattern = if fields.is_empty() {
                        ctor.name.clone()
                    } else {
                        format!("{} {{ {} }}", ctor.name, fields.join(", "))
                    };
                    let e = if cs.contains(c) {
                        format!("({body})")
                    } else {
                        "true".to_string()
                    };
                    format!("{pattern} -> {e}")
                })
                .collect();
            edits.push(Edit::new(
                j,
                pred,
                Term::text(&format!("match {f} | {}", arms.join(" | "))),
            ));
        }
        Ok(edits)
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
            let Some(held) = self.relations[&cl.relation].index_at(cl.at) else {
                return broken("a scoped clause that is no index");
            };
            let mut target: Option<String> = None;
            let mut fs = Vec::new();
            for (g, e) in cl.elems.iter().zip(held.elems().iter().map(Elem::term)) {
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
