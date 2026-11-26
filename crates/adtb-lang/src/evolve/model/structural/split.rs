use super::{Routing, carried, ctor_in, def_in};
use crate::ast::{self, CtorScope, SplitClause};
use crate::capability::RootKind;
use crate::check::{self, RowBody};
use crate::evolve::RowFn;
use crate::evolve::carry::ctor_fields;
use crate::evolve::carry::{self, Structural};
use crate::evolve::ctor::{Carries, arm_image, element_head, matched_fields};
use crate::evolve::model::admin::canonical;
use crate::evolve::model::defaults::Taken;
use crate::evolve::model::field::Applied;
use crate::evolve::model::indexes::{Held, NewIndex};
use crate::evolve::model::op::{Fail, Res, broken, refuse, refuse_at, refuse_first};
use crate::evolve::model::reselect::Mode;
use crate::evolve::model::run::{Built, Late, Run};
use crate::evolve::model::*;
use crate::evolve::{Clause, Cx, Resolved, RowMap};
use crate::schema::Schema;
use crate::span::FileId;
use crate::types::{KeyCtors, RelRef, Ty, TypeId};

/// Returns how a unique or index of kind `kind` is written.
pub(super) fn kind_word(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Unique => "unique",
        IndexKind::Plain => "index",
        IndexKind::Interval => "interval index",
    }
}

/// Returns the names of scope `s`.
fn scope_names(s: &CtorScope) -> Vec<String> {
    match s {
        CtorScope::One(c) => vec![c.text.clone()],
        CtorScope::Many(cs) => cs.iter().map(|c| c.text.clone()).collect(),
    }
}

/// Returns a selection of `names` as written: `C`, or `(C | D)`.
fn sel_text(names: &[String]) -> String {
    match names {
        [one] => one.clone(),
        many => format!("({})", many.join(" | ")),
    }
}

/// Returns an index clause of kind `kind` over the elements `elems` in scope `names`, as text
/// a build resolves.
fn index_clause(kind: IndexKind, elems: &[String], names: &[String], was: Vec<u64>) -> NewIndex {
    NewIndex::new(
        kind,
        elems.iter().map(|e| Term::text(e)).collect(),
        Some(names.iter().map(|n| Term::text(n)).collect()),
        was,
    )
}

/// A clause a split adds to a relation variable: its text and the clause.
type Added = (String, NewIndex);

/// How a split rewrites the unique and index clauses: by relation variable, the positions of
/// those it removes and the clauses it appends, and the place among those appended of each
/// replacement the split states; and those it rescopes in place.
#[derive(Default)]
struct Rewrites {
    clauses_of: BTreeMap<u64, (BTreeSet<usize>, Vec<Added>)>,
    replaced_at: BTreeMap<u64, Vec<usize>>,
    rescoped: Vec<(u64, usize, Vec<Term>)>,
    /// The clauses that keep their scope and id, by relation variable and position, with their
    /// elements over the products.
    carried: Vec<(u64, usize, Vec<Term>)>,
}

/// A split of constructor `ctor`, named `source`, into `products`, whose fields `shapes` gives.
struct Image<'a> {
    ctor: u64,
    source: String,
    products: Vec<String>,
    shapes: Vec<(String, Vec<String>)>,
    carries: &'a Carries,
    /// The names of the header fields.
    header: Vec<String>,
}

impl Image<'_> {
    fn is_header(&self, f: &str) -> bool {
        self.header.iter().any(|h| h == element_head(f).0)
    }

    /// Returns the first element of `elems` some product does not keep under its name, with
    /// that product.
    fn unkept<'e>(&self, elems: &'e [String]) -> Option<(&'e String, &String)> {
        elems.iter().find_map(|f| {
            if self.is_header(f) {
                return None;
            }
            let head = element_head(f).0;
            let keeps = |(p, fs): &&(String, Vec<String>)| {
                fs.iter().any(|g| g == head)
                    && !self
                        .carries
                        .iter()
                        .any(|(x, y, q, z)| *x == self.source && y == head && q == p && z != head)
            };
            self.shapes.iter().find(|s| !keeps(s)).map(|(p, _)| (f, p))
        })
    }

    /// Returns `elems` as product `p` holds them, `None` when it keeps not every one.
    fn image(&self, elems: &[String], p: &str) -> Option<Vec<String>> {
        elems
            .iter()
            .map(|f| {
                let (head, rest) = element_head(f);
                if self.is_header(f) {
                    return Some(f.clone());
                }
                self.carries
                    .iter()
                    .find(|(x, y, z, _)| *x == self.source && y == head && z == p)
                    .map(|(_, _, _, g)| format!("{g}{rest}"))
            })
            .collect()
    }
}

/// Returns the replacements `given` states, each with its text, and its arms.
fn replacements(run: &mut Run, given: &[SplitClause]) -> (Vec<Added>, Vec<ast::ArmClause>) {
    let mut out = Vec::new();
    let mut arms = Vec::new();
    for cl in given {
        let (kind, fields, scope) = match cl {
            SplitClause::Unique(fs, s) => (IndexKind::Unique, fs, s),
            SplitClause::Index {
                interval: true,
                fields,
                scope,
            } => (IndexKind::Interval, fields, scope),
            SplitClause::Index { fields, scope, .. } => (IndexKind::Plain, fields, scope),
            SplitClause::Arm(a) => {
                arms.push((**a).clone());
                continue;
            }
        };
        let span = fields[0].span.to(fields[fields.len() - 1].span);
        let elems = &run.text()[span.range()];
        let names = scope_names(scope);
        out.push((
            format!("{} ({elems}) in {}", kind_word(kind), sel_text(&names)),
            index_clause(kind, &[elems.to_string()], &names, Vec::new()),
        ));
    }
    (out, arms)
}

impl Model {
    /// Check routing `arms` over the rows of `sources` of type `t` against the products `made`,
    /// in the schema the operator gives with `sources` kept: `made` after the other
    /// constructors, every selection and reference to `sources` outside a check put to them,
    /// with `whole` as the operator's, the arms of `given` in the bodies, checked whole, and no
    /// unique or index on a relation variable of `t`. `built` resolves the declarations before
    /// the operator.
    ///
    /// Preconditions:
    /// - `made` are constructors of `t` that its body does not yet list.
    pub(super) fn routing(
        &self,
        run: &mut Run,
        built: &Built,
        (t, sources, made): (u64, &[u64], &[u64]),
        (arms, given): (&[ast::Arm], &[ast::ArmClause]),
        whole: bool,
    ) -> Res<Routing> {
        let mut probe = self.clone();
        probe.reselect_products(run, t, sources, made)?;
        if let TypeBody::Ctors(cs) = &mut probe.types.get_mut(&t).expect("the type").body {
            cs.extend(made);
        }
        let mut edits = probe.reselect(
            built,
            t,
            sources,
            made,
            Mode::structural("the routing", false, false),
        )?;
        edits.extend(probe.reselect_rows(built, t, (sources, made), "the routing", whole)?);
        edits.extend(probe.arm_writes(run, built, t, (sources, &[]), given)?.0);
        probe.edit_decls(edits)?;
        for (r, ..) in probe.rels_of(built, t) {
            probe
                .relations
                .get_mut(&r)
                .expect("a relation variable")
                .drop_indexes();
        }
        let compiled = probe.checked_without_dependents(run)?;
        let schema = compiled.schema();
        let def = def_in(schema, t);
        let module = schema
            .module_named(self.module_of((Space::Shared, t)).expect("a declared type"))
            .expect("the type's module");
        let src: Vec<u32> = sources.iter().map(|&c| ctor_in(schema, c)).collect();
        let prod: Vec<u32> = made.iter().map(|&c| ctor_in(schema, c)).collect();
        let row = Cx::row_ty(schema, def, Some(src));
        let result = Cx::row_ty(schema, def, Some(prod));
        let span = run.span();
        let body = || RowBody::Match(arms, span);
        let refs = check::check_row(
            &compiled,
            module,
            &row,
            body(),
            Some(&result),
            RootKind::Routing,
        )
        .or_else(|d| refuse_at(d.span, d.message))?;
        let text = match (arms.first(), arms.last()) {
            (Some(a), Some(z)) => canonical(run, schema, &refs, a.patterns[0].span.to(z.body.span)),
            _ => String::new(),
        };
        let q = check::row_query(
            &compiled,
            module,
            &row,
            body(),
            Some(&result),
            RootKind::Routing,
        )
        .or_else(|d| refuse_at(d.span, d.message))?;
        let query = crate::norm::registered(&compiled, &q.terms, q.term, RootKind::Query)
            .map_err(|e| Fail::Broken(format!("a routing does not normalise: {e:?}")))?;
        Ok(Routing {
            rows: RowFn {
                schema: schema.clone(),
                query,
            },
            text,
        })
    }

    /// Returns the schema the model builds now and its program, without each function,
    /// aggregate, constraint and check that does not check, refusing the operator with the
    /// first failure of any other declaration.
    fn checked_without_dependents(mut self, run: &mut Run) -> Res<check::Compiled<Schema>> {
        loop {
            let ds = match self.schema(&run.ahead(&self)).map(check::check_schema) {
                Ok(Ok(c)) => return Ok(c),
                Ok(Err(ds)) | Err(ds) => ds,
            };
            let failing: Vec<Item> = ds
                .iter()
                .filter_map(|d| self.decls.get(d.file.0 as usize))
                .map(|d| d.item)
                .filter(|i| {
                    matches!(
                        i,
                        Item::Fn(_) | Item::Aggregate(_) | Item::Constraint(_) | Item::Check(_)
                    )
                })
                .collect();
            if failing.is_empty() {
                return refuse_first(&ds, str::to_string);
            }
            for item in failing {
                self.remove(item);
            }
        }
    }

    /// Put the products `made` in place of the constructors `sources` of type `t` in the
    /// reference types of the products' own fields.
    ///
    /// Preconditions:
    /// - `made` are constructors of `t` that its body does not yet list.
    pub(super) fn reselect_products(
        &mut self,
        run: &mut Run,
        t: u64,
        sources: &[u64],
        made: &[u64],
    ) -> Res<()> {
        let mut probe = self.clone();
        if let TypeBody::Ctors(cs) = &mut probe.types.get_mut(&t).expect("the type").body {
            cs.extend(made);
        }
        let Ok((pre, _)) = probe.built(&[], &run.ahead(&probe)) else {
            return Ok(());
        };
        let ps = pre.schema();
        let def = def_in(ps, t);
        let src: Vec<u32> = sources.iter().map(|&c| ctor_in(ps, c)).collect();
        let names_source = made.iter().any(|&p| {
            ps.ty(def).ctors()[ctor_in(ps, p) as usize]
                .fields
                .iter()
                .any(|f| names_ctor(ps, &f.ty, def, &src))
        });
        if !names_source {
            return Ok(());
        }
        let edits = probe.reselect(
            &pre,
            t,
            sources,
            made,
            Mode::structural("the split", false, false),
        )?;
        probe.edit_decls(edits)?;
        for &p in made {
            for f in self.ctors[&p].fields.clone().unwrap_or_default() {
                self.fields.get_mut(&f).expect("a product's field").ty =
                    probe.fields[&f].ty.clone();
            }
        }
        Ok(())
    }

    pub(in crate::evolve::model) fn split(
        &mut self,
        run: &mut Run,
        (t, c): (u64, u64),
        path: &ast::CtorPath,
        into: &[ast::CtorDef],
        arms: &[ast::Arm],
        given: &[SplitClause],
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        for d in into {
            self.fresh_ctor(t, d)?;
        }
        self.paths_image(t, &[c], "the split")?;
        let (replacements, arm_clauses) = replacements(run, given);
        let carries = carried(arms);
        let made: Vec<u64> = into
            .iter()
            .map(|d| self.new_ctor(run, t, &d.name.text, d.record.as_deref()))
            .collect();
        self.defaults_image(
            &built,
            t,
            (&[c], &made),
            "the split",
            Taken::Split(&carries),
        )?;
        self.reselect_products(run, t, &[c], &made)?;
        let products: Vec<String> = into.iter().map(|d| d.name.text.clone()).collect();
        let plan = self.carry_plan(run, &built, Structural::Split, t, (&[c], &products));
        let mut edits = Vec::new();
        edits.extend(self.reselect_rows(&built, t, (&[c], &made), "the split", false)?);
        let image = Image {
            ctor: c,
            source: path.ctor.text.clone(),
            products: into.iter().map(|d| d.name.text.clone()).collect(),
            shapes: into.iter().map(ctor_fields).collect(),
            carries: &carries,
            header: self.header_names(t),
        };
        let mut rewrites = self.split_clauses(&built, t, &made, &image, &replacements)?;
        for (r, (_, added)) in &mut rewrites.clauses_of {
            let mut at = Vec::new();
            for (text, clause) in &replacements {
                match added.iter().position(|(x, _)| x == text) {
                    Some(k) => {
                        added[k].1 = clause.clone();
                        at.push(k);
                    }
                    None => {
                        at.push(added.len());
                        added.push((text.clone(), clause.clone()));
                    }
                }
            }
            rewrites.replaced_at.insert(*r, at);
        }
        let (arm_edits, written) = self.arm_writes(run, &built, t, (&[c], &[c]), &arm_clauses)?;
        edits.extend(arm_edits);
        edits.extend(self.reselect(
            &built,
            t,
            &[c],
            &made,
            Mode::structural("the split", false, true),
        )?);
        let stages = self.restated(&built, t, &[c], false)?;
        let td = schema.ty(def_in(schema, t));
        let readable: Vec<String> = td
            .header()
            .iter()
            .chain(&td.ctors()[ctor_in(schema, c) as usize].fields)
            .map(|f| f.name.clone())
            .collect();
        let images: Vec<_> = arms.iter().map(arm_image).collect();
        let routed = carry::routed(&images, std::slice::from_ref(&image.source), &image.shapes);
        let routing = self.routing(run, &built, (t, &[c], &made), (arms, &arm_clauses), false)?;
        let before = self.clone();
        self.edit_decls(edits)?;
        let first = self
            .rels_of(&built, t)
            .into_iter()
            .map(|r| r.0)
            .find(|r| rewrites.clauses_of.contains_key(r));
        let (rels, replaced_at) = self.rewrite_split(rewrites, first);
        self.identify();
        self.replace_ctors(t, &[c], &made);
        self.rebind(&before);
        self.restate(&made, &stages);
        let after = self.rebuilt(run, t, &rels)?;
        let armed = self.arm_clauses(run, after.as_ref().map_err(Vec::as_slice), written)?;
        let after = after.ok();
        let record = into.last().is_some_and(|d| d.record.is_some());
        let (decl, clauses) = match &after {
            Some(after) => (
                self.products_canonical(after, run, t, &made, record)?,
                self.split_resolved(run, after, armed, given, (first, replaced_at))?,
            ),
            None => {
                if !replaced_at.is_empty() {
                    run.late(Late::Unresolved);
                }
                (
                    run.late(Late::Ctors {
                        first: made[0],
                        last: made[made.len() - 1],
                        record,
                    }),
                    armed,
                )
            }
        };
        self.settle_ids(run);
        self.carry_grants(
            t,
            plan,
            (&routed.0, &routed.1),
            &[],
            &matched_fields(arms, &readable),
        );
        Ok((
            Resolved::Split {
                ctor: c,
                decl,
                routing: routing.text,
            },
            clauses,
            Ok(vec![RowMap::Route {
                def: def_in(schema, t),
                from: vec![ctor_in(schema, c)],
                routing: routing.rows,
            }]),
        ))
    }

    /// Returns how a split rewrites each unique and index clause over its source, as `built`
    /// resolves the declarations before it; `made` are its products.
    fn split_clauses(
        &self,
        built: &Built,
        t: u64,
        made: &[u64],
        image: &Image,
        replacements: &[Added],
    ) -> Res<Rewrites> {
        let mut out = Rewrites::default();
        let (c, source, carries) = (image.ctor, &image.source, image.carries);
        let product_id = |n: &String| {
            let at = image.products.iter().position(|p| p == n);
            made[at.expect("a product of the split")]
        };
        for cl in self.scoped(built, t, &[c], made) {
            out.clauses_of.entry(cl.relation).or_default();
            let Some(reached) = &cl.reached else { continue };
            if !reached.contains(&c) {
                continue;
            }
            let shown = format!("{} ({})", kind_word(cl.kind), cl.elems.join(", "));
            if reached.len() > 1 {
                if let Some((f, p)) = image.unkept(&cl.elems) {
                    if replacements.is_empty() {
                        let written = self.scope_written(cl.relation, cl.at);
                        return refuse(format!(
                            "`{shown} in {}` names `{f}`, which `{p}` does not keep under that name",
                            sel_text(&written)
                        ));
                    }
                    out.clauses_of
                        .entry(cl.relation)
                        .or_default()
                        .0
                        .insert(cl.at);
                } else if let Some(parts) = &cl.rescoped {
                    out.rescoped.push((cl.relation, cl.at, parts.clone()));
                }
                continue;
            }
            let written = sel_text(&self.scope_written(cl.relation, cl.at));
            let lost = cl.elems.iter().rev().find(|f| {
                let head = element_head(f).0;
                !image.is_header(f) && !carries.iter().any(|(x, y, _, _)| x == source && y == head)
            });
            if let Some(f) = lost
                && replacements.is_empty()
            {
                return refuse(format!(
                    "`{shown} in {written}` names `{f}`, which no new constructor keeps; state its replacements after the routing"
                ));
            }
            if cl.rescoped.is_none() {
                let Some(elems) = self.carried_elems(t, (cl.relation, cl.at), image, made) else {
                    return refuse(format!(
                        "`{shown} in {written}` has no image under the split: its products do not hold its elements under one name"
                    ));
                };
                out.carried.push((cl.relation, cl.at, elems));
                continue;
            }
            let entry = out.clauses_of.entry(cl.relation).or_default();
            entry.0.insert(cl.at);
            let images: Vec<Option<Vec<String>>> = image
                .products
                .iter()
                .map(|p| image.image(&cl.elems, p))
                .collect();
            let apart: Vec<(Vec<String>, Vec<String>)> = match images.first() {
                Some(Some(fs)) if images.iter().all(|x| x.as_ref() == Some(fs)) => {
                    vec![(fs.clone(), image.products.clone())]
                }
                _ if !replacements.is_empty() => continue,
                _ if cl.kind != IndexKind::Unique && images.iter().all(Option::is_some) => images
                    .into_iter()
                    .flatten()
                    .zip(&image.products)
                    .map(|(fs, p)| (fs, vec![p.clone()]))
                    .collect(),
                _ => {
                    return refuse(format!(
                        "`{shown} in {written}` has no image under the split: its products do not hold its elements under one name"
                    ));
                }
            };
            for (fs, names) in apart {
                let text = format!(
                    "{} ({}) in {}",
                    kind_word(cl.kind),
                    fs.join(", "),
                    sel_text(&names)
                );
                let was = cl.id.into_iter().collect::<Vec<u64>>();
                match entry.1.iter_mut().find(|(x, _)| *x == text) {
                    Some((_, held)) => held.also_image_of(was),
                    None => {
                        let ids: Vec<u64> = names.iter().map(&product_id).collect();
                        let Some(elems) = self.product_elems(t, (cl.relation, cl.at), &fs, &ids)
                        else {
                            return broken("a product holds each field the routing carries to it");
                        };
                        let parts = ids
                            .iter()
                            .zip(&names)
                            .map(|(&p, n)| Term::name((Space::Shared, p), n))
                            .collect();
                        entry
                            .1
                            .push((text, NewIndex::new(cl.kind, elems, Some(parts), was)));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Apply `rw` to the relation variables. Returns the relation variables it rewrites, and the
    /// positions `first` of them gives the split's replacements, in order.
    fn rewrite_split(&mut self, mut rw: Rewrites, first: Option<u64>) -> (Vec<u64>, Vec<usize>) {
        let rels: Vec<u64> = rw
            .clauses_of
            .keys()
            .copied()
            .chain(rw.rescoped.iter().map(|r| r.0))
            .collect();
        for (r, at, carried) in rw.carried {
            let rel = self.relations.get_mut(&r).expect("a relation variable");
            rel.carry_elems(at, carried);
        }
        for (r, at, parts) in rw.rescoped {
            let rel = self.relations.get_mut(&r).expect("a relation variable");
            rel.rescope(at, parts);
        }
        let mut replaced_at: Vec<usize> = Vec::new();
        for (r, (gone, added)) in rw.clauses_of {
            let rel = self.relations.get_mut(&r).expect("a relation variable");
            rel.remove_clauses(&gone);
            let base = rel.clauses.len();
            if Some(r) == first {
                let at = rw.replaced_at.remove(&r).unwrap_or_default();
                replaced_at = at.into_iter().map(|k| base + k).collect();
            }
            for (_, c) in added {
                rel.push_index(c);
            }
        }
        (rels, replaced_at)
    }

    /// Returns the clauses of a split's resolved form, in the order `given` states them: each
    /// replacement with its index, as `after` the split writes it, and each of `arms`.
    fn split_resolved(
        &mut self,
        run: &mut Run,
        after: &Built,
        arms: Vec<Clause>,
        given: &[SplitClause],
        (first, replaced_at): (Option<u64>, Vec<usize>),
    ) -> Res<Vec<Clause>> {
        let mut replaced = replaced_at.into_iter();
        let mut arms = arms.into_iter();
        let mut out = Vec::new();
        for cl in given {
            let kind = match cl {
                SplitClause::Arm(_) => {
                    out.extend(arms.next());
                    continue;
                }
                SplitClause::Unique(..) => crate::evolve::IndexKind::Unique,
                SplitClause::Index { interval: true, .. } => crate::evolve::IndexKind::Interval,
                SplitClause::Index { .. } => crate::evolve::IndexKind::Plain,
            };
            let (Some(r), Some(at)) = (first, replaced.next()) else {
                return broken("a split's replacement without a new index");
            };
            let Some(index) = self.relations[&r].index_at(at).and_then(Held::id) else {
                return broken("a split's replacement without a new index");
            };
            out.push(Clause::Replacement {
                index,
                kind,
                fields: self.elems_canonical(after, run, first, at)?,
            });
        }
        Ok(out)
    }

    /// Returns the elements of clause `at` of relation variable `r` of type `t` over the split's
    /// products `made`, each held by the field of every product the routing carries the
    /// source's field to; `None` when the products do not hold them under one name.
    fn carried_elems(
        &self,
        t: u64,
        (r, at): (u64, usize),
        image: &Image,
        made: &[u64],
    ) -> Option<Vec<Term>> {
        let held = self.relations[&r].index_at(at)?;
        let printed: Vec<String> = held.elems().iter().map(|e| e.print(self)).collect();
        let mut images = image.products.iter().map(|p| image.image(&printed, p));
        let first = images.next()??;
        if images.any(|x| x.as_ref() != Some(&first)) {
            return None;
        }
        self.product_elems(t, (r, at), &first, made)
    }

    /// Returns the scope of clause `at` of relation variable `r` as written.
    fn scope_written(&self, r: u64, at: usize) -> Vec<String> {
        self.relations[&r]
            .index_at(at)
            .map(|h| h.scope_names(self))
            .unwrap_or_default()
    }

    /// Returns the canonical form of the constructors `made` of type `t` as `built` declares
    /// them, from the first's name through the last's record when `record`.
    fn products_canonical(
        &self,
        built: &Built,
        run: &mut Run,
        t: u64,
        made: &[u64],
        record: bool,
    ) -> Res<String> {
        let file = FileId(self.decl_at(Item::Type(t)).expect("the type") as u32);
        Ok(self
            .ctors_written(built, file, (made[0], made[made.len() - 1]), record)?
            .unwrap_or_else(|| run.late(Late::Unresolved)))
    }

    /// Returns the canonical form of the elements of clause `at` of relation variable `r` as
    /// `built` declares it.
    fn elems_canonical(
        &self,
        built: &Built,
        run: &mut Run,
        r: Option<u64>,
        at: usize,
    ) -> Res<String> {
        let Some(r) = r else {
            return broken("a split's replacement with no relation variable");
        };
        let file = FileId(self.decl_at(Item::Relation(r)).expect("the relation") as u32);
        let Some(ast::Decl::Relation(rd)) = built.decl(file) else {
            return broken("a relation the model printed does not parse");
        };
        let Some(
            ast::RelationClause::Unique(fields, ..) | ast::RelationClause::Index { fields, .. },
        ) = rd.clauses.get(at)
        else {
            return broken("a replacement the model printed as another clause");
        };
        let span = fields[0].span.to(fields[fields.len() - 1].span);
        Ok(Self::canonical_at(built, run, file, span))
    }
}

/// Returns whether type `ty` names a constructor of `cs` of type `def`.
fn names_ctor(schema: &Schema, ty: &Ty, def: TypeId, cs: &[u32]) -> bool {
    let named = |set: &[u32]| set.iter().any(|c| cs.contains(c));
    match ty {
        Ty::Key {
            rel: RelRef::Declared(r),
            ctors: KeyCtors::Only(set),
        } => schema.relation(*r).elem_def == def && named(set),
        Ty::Data {
            def: d,
            args,
            ctors,
        } => {
            (*d == def && ctors.as_deref().is_some_and(named))
                || args.iter().any(|a| names_ctor(schema, a, def, cs))
        }
        Ty::Set(a) | Ty::NonEmpty(a) | Ty::Tag(a) => names_ctor(schema, a, def, cs),
        Ty::Group(a, b) => names_ctor(schema, a, def, cs) || names_ctor(schema, b, def, cs),
        Ty::Record(fs) => fs.iter().any(|(_, f)| names_ctor(schema, f, def, cs)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, f)| names_ctor(schema, f, def, cs))),
        _ => false,
    }
}
