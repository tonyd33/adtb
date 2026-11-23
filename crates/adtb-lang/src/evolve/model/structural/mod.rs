mod merge;
mod nest;
mod split;

use super::op::{Res, broken, refuse, refuse_at};
use super::reselect::{Edit, reselected, sel_range};
use super::run::{Built, Run};
use super::*;
use crate::ast::{self, CtorSel};
use crate::check::{self, Ref};
use crate::evolve::RowFn;
use crate::evolve::carry::{self, Carry, Plan, Structural};
use crate::evolve::ctor::Carries;
use crate::schema::{Schema, Stage};
use crate::span::FileId;
use crate::types::TypeId;

/// The write state each relation variable of a type, by id, and the type itself, as `None`,
/// give the constructors a structural operator makes.
pub(super) type Stages = Vec<(Option<u64>, Stage)>;

/// Returns the type of `t` in `schema`.
pub(super) fn def_in(schema: &Schema, t: u64) -> TypeId {
    schema
        .ids()
        .type_of(cat_type(t))
        .expect("a type the model holds")
}

/// Returns the position of constructor `c` in its type in `schema`.
pub(super) fn ctor_in(schema: &Schema, c: u64) -> u32 {
    schema
        .ids()
        .ctor_of(cat_ctor(c))
        .expect("a constructor the model holds")
        .index()
}

/// Returns the names of a constructor selection.
pub(super) fn sel_idents(sel: &CtorSel) -> Vec<&ast::Ident> {
    match sel {
        CtorSel::One(c) => vec![c],
        CtorSel::Many(cs) => cs.iter().collect(),
    }
}

impl Model {
    /// Returns the clause of a relation variable over `t` whose scope reaches the constructors
    /// `gone`, by relation and position, with its kind, elements as printed, scope reached and
    /// scope rewritten with `by` in place of each of `gone` it lists, `None` when it lists none.
    fn scoped(&self, built: &Built, t: u64, gone: &[u64], by: &[u64]) -> Vec<Scoped> {
        let mut out = Vec::new();
        for (r, ..) in self.rels_of(built, t) {
            for (k, c) in self.relations[&r].clauses.iter().enumerate() {
                let ClauseM::Index {
                    kind, elems, scope, ..
                } = c
                else {
                    continue;
                };
                let reached = scope.as_ref().map(|s| self.scope_ctors(s));
                let rescoped = scope.as_ref().and_then(|s| {
                    let lists = s.parts.iter().any(|p| {
                        p.objects()
                            .next()
                            .is_some_and(|o| o.0 == Space::Shared && gone.contains(&o.1))
                    });
                    lists.then(|| {
                        let mut parts: Vec<Term> = Vec::new();
                        let mut seen: Vec<Obj> = Vec::new();
                        for p in &s.parts {
                            let objs: Vec<Obj> = match p.objects().next() {
                                Some(o) if o.0 == Space::Shared && gone.contains(&o.1) => {
                                    by.iter().map(|&b| (Space::Shared, b)).collect()
                                }
                                Some(o) => vec![o],
                                None => Vec::new(),
                            };
                            for o in objs {
                                if seen.contains(&o) {
                                    continue;
                                }
                                seen.push(o);
                                parts.push(if o.0 == Space::Shared && by.contains(&o.1) {
                                    Term::name(o, &self.ctors[&o.1].name)
                                } else {
                                    p.clone()
                                });
                            }
                        }
                        parts
                    })
                });
                out.push(Scoped {
                    relation: r,
                    at: k,
                    kind: *kind,
                    elems: elems.iter().map(|e| e.print(self)).collect(),
                    reached,
                    rescoped,
                });
            }
        }
        out
    }

    /// Returns the constructors of type `t` that `sel` names, in declaration order.
    pub(super) fn sources(&self, t: u64, sel: &CtorSel) -> Res<Vec<u64>> {
        let all = self.ctors_of(t);
        let mut at = Vec::new();
        for x in sel_idents(sel) {
            match all.iter().position(|c| self.ctors[c].name == x.text) {
                Some(k) => at.push(k),
                None => {
                    return refuse_at(
                        x.span,
                        format!(
                            "`{}` is not a constructor of `{}`",
                            x.text, self.types[&t].name
                        ),
                    );
                }
            }
        }
        at.sort_unstable();
        at.dedup();
        Ok(at.into_iter().map(|k| all[k]).collect())
    }

    /// Refuse a constructor named `d`'s name that type `t` already has.
    pub(super) fn fresh_ctor(&self, t: u64, d: &ast::CtorDef) -> Res<()> {
        if self.ctor_named(t, &d.name.text).is_some() {
            return refuse_at(
                d.name.span,
                format!(
                    "`{}` is already a constructor of `{}`",
                    d.name.text, self.types[&t].name
                ),
            );
        }
        Ok(())
    }

    /// Returns the write states the constructors replacing `sources` of type `t` take, for each
    /// relation variable of `t` in `built`'s order and then the type. With `merge` a product is
    /// deprecated when every source is, declared when a source is, else enabled, and a
    /// deprecated source among others refuses; otherwise it takes the first source's state.
    pub(super) fn restated(
        &self,
        built: &Built,
        t: u64,
        sources: &[u64],
        merge: bool,
    ) -> Res<Stages> {
        let combine = |states: &[Stage]| -> Option<Stage> {
            if !merge {
                return Some(states.first().copied().unwrap_or(Stage::Enabled));
            }
            if states.contains(&Stage::Deprecated) {
                return states
                    .iter()
                    .all(|s| *s == Stage::Deprecated)
                    .then_some(Stage::Deprecated);
            }
            Some(if states.contains(&Stage::Declared) {
                Stage::Declared
            } else {
                Stage::Enabled
            })
        };
        let mut owners: Vec<(Option<u64>, String)> = self
            .rels_of(built, t)
            .into_iter()
            .map(|(r, _, name)| (Some(r), name))
            .collect();
        owners.push((None, self.types[&t].name.clone()));
        let mut out = Vec::new();
        for (owner, shown) in owners {
            let state = |c: u64| match owner {
                Some(r) => self.states.get(&(r, c)),
                None => self.value_states.get(&c),
            };
            let of: Vec<Stage> = sources
                .iter()
                .map(|&c| state(c).map_or(Stage::Enabled, |s| s.stage))
                .collect();
            let Some(stage) = combine(&of) else {
                let which = sources
                    .iter()
                    .find(|&&c| state(c).is_some_and(|s| s.stage == Stage::Deprecated))
                    .map(|c| self.ctors[c].name.as_str())
                    .unwrap_or_default();
                return refuse(format!(
                    "merging or nesting a deprecated constructor with one in another state is refused: `{shown}.{which}` is deprecated"
                ));
            };
            out.push((owner, stage));
        }
        Ok(out)
    }

    /// Give each of `products` the write states `stages`, since this migration.
    pub(super) fn restate(&mut self, products: &[u64], stages: &Stages) {
        let since = self.version;
        for &p in products {
            for &(owner, stage) in stages {
                let state = State { stage, since };
                match owner {
                    Some(r) => self.states.insert((r, p), state),
                    None => self.value_states.insert(p, state),
                };
            }
        }
    }

    /// Put `made` in the place of the first of `sources` among the constructors of type `t`, and
    /// remove `sources` with their fields.
    pub(super) fn replace_ctors(&mut self, t: u64, sources: &[u64], made: &[u64]) {
        let TypeBody::Ctors(cs) = &mut self.types.get_mut(&t).expect("the type").body else {
            return;
        };
        let at = cs
            .iter()
            .position(|c| sources.contains(c))
            .unwrap_or(cs.len());
        for (k, &m) in made.iter().enumerate() {
            cs.insert(at + k, m);
        }
        for &s in sources {
            if !made.contains(&s) {
                self.drop_ctor(t, s);
            }
        }
    }

    /// Add constructor `name` with fields `fields` written in `run`, to type `t`'s table of
    /// constructors only. Returns its id.
    pub(super) fn new_ctor(
        &mut self,
        run: &mut Run,
        t: u64,
        name: &str,
        fields: Option<&[ast::FieldType]>,
    ) -> u64 {
        let c = self.fresh(Space::Shared);
        let fields = fields.map(|fs| {
            fs.iter()
                .map(|f| {
                    let id = self.fresh(Space::Field);
                    self.fields.insert(
                        id,
                        FieldM {
                            owner: c,
                            name: f.name.text.clone(),
                            ty: Term::text(&run.text()[f.ty.span.range()]),
                            action: f
                                .on_delete
                                .as_ref()
                                .map(|a| Term::text(&run.text()[a.span.range()])),
                        },
                    );
                    id
                })
                .collect()
        });
        self.ctors.insert(
            c,
            CtorM {
                ty: t,
                name: name.to_string(),
                fields,
            },
        );
        c
    }

    /// Returns how a structural operator `what` replacing `sources` of type `t` with `products`,
    /// all by name, carries grants, as `built`, before it, resolves the declarations.
    pub(super) fn carry_plan(
        &self,
        run: &mut Run,
        built: &Built,
        what: Structural,
        t: u64,
        (sources, products): (&[u64], &[String]),
    ) -> Plan {
        let relations: Vec<(u64, String)> = self
            .rels_of(built, t)
            .into_iter()
            .map(|(r, _, n)| (r, n))
            .collect();
        let mut sets = BTreeMap::new();
        for (r, _) in &relations {
            for s in self.sets_of(*r) {
                let held = self.sets[&s]
                    .members
                    .iter()
                    .filter_map(|m| m.objects().next().map(|o| o.1))
                    .filter(|c| sources.contains(c))
                    .collect();
                sets.insert(s, held);
            }
        }
        let mut fields = BTreeMap::new();
        let header = self.types[&t].header.iter().flatten();
        for &s in sources {
            for f in header.clone().chain(self.ctors[&s].fields.iter().flatten()) {
                fields.insert((s, self.fields[f].name.clone()), *f);
            }
        }
        Plan::new(
            what,
            run.span(),
            (relations, sets),
            (
                sources
                    .iter()
                    .map(|&s| (s, self.ctors[&s].name.clone()))
                    .collect(),
                products.to_vec(),
                fields,
            ),
        )
    }

    /// Carry the grants across a structural operator by `plan`, whose products of type `t` the
    /// model now holds, and note those it could not carry.
    pub(super) fn carry_grants(
        &mut self,
        t: u64,
        plan: Plan,
        routed: (&carry::Reaches, &carry::Given),
        discriminators: &[String],
        matched: &[String],
    ) {
        if self.grants.is_empty() {
            return;
        }
        let products: Vec<(String, u64)> = plan
            .products()
            .iter()
            .filter_map(|p| Some((p.clone(), self.ctor_named(t, p)?)))
            .collect();
        let carry = Carry::new(plan, routed, discriminators, matched);
        let field_id = |p: &str, g: &str| {
            let c = self.ctor_named(t, p)?;
            self.ctors[&c]
                .fields
                .iter()
                .flatten()
                .find(|f| self.fields[f].name == g)
                .copied()
        };
        let mut grants = self.grants.clone();
        let unstated = carry.carry(&mut grants, &products, &field_id);
        self.grants = grants;
        for u in unstated {
            if !self.unstated.iter().any(|x| x.same(&u)) {
                self.unstated.push(u);
            }
        }
    }

    /// Returns the edits that put `by` in place of `gone` of type `t` in every selection an
    /// expression or a row type makes, such as `from l in places.Location` or
    /// `(p: Place.Location)`, as `built` resolves the declarations. With `whole`, a selection
    /// of only some of `gone` is left as it is.
    pub(super) fn reselect_rows(
        &self,
        built: &Built,
        t: u64,
        gone: &[u64],
        by: &[u64],
        whole: bool,
    ) -> Vec<Edit> {
        let schema = built.schema();
        let def = def_in(schema, t);
        let gone_ix: Vec<u32> = gone.iter().map(|&c| ctor_in(schema, c)).collect();
        let noted: BTreeSet<(FileId, u32)> = check::schema_refs(schema)
            .into_iter()
            .filter(|(_, _, r)| matches!(r, Ref::Ctor(d, c) if *d == def && gone_ix.contains(c)))
            .map(|(f, s, _)| (f, s.start))
            .collect();
        let tname = &self.types[&t].name;
        let tmodule = self.module_of((Space::Shared, t)).unwrap_or_default();
        let mut out: Vec<Edit> = Vec::new();
        for j in 0..self.decls.len() {
            let file = FileId(j as u32);
            let Some(d) = built.decl(file) else {
                continue;
            };
            let home = &self.decls[j].module;
            let mut sels: Vec<Vec<ast::Ident>> = Vec::new();
            for e in crate::evolve::text::decl_exprs(d) {
                crate::visit::each_expr(e, &mut |x| {
                    if let ast::ExprKind::Select(_, sel) = &x.kind
                        && sel_idents(sel)
                            .iter()
                            .any(|i| noted.contains(&(file, i.span.start)))
                    {
                        sels.push(sel_idents(sel).into_iter().cloned().collect());
                    }
                });
            }
            crate::evolve::each_type(d, &mut |ty| {
                if let ast::TypeKind::Named {
                    name,
                    sel: Some(sel),
                    ..
                } = &ty.kind
                    && name.name.text == *tname
                    && name
                        .module
                        .as_ref()
                        .map_or(home == tmodule, |m| m.text == tmodule)
                {
                    sels.push(sel_idents(sel).into_iter().cloned().collect());
                }
            });
            let h = format!("module {home}\n").len();
            for idents in sels {
                let Some(names) = idents
                    .iter()
                    .map(|x| self.ctor_named(t, &x.text))
                    .collect::<Option<Vec<u64>>>()
                else {
                    continue;
                };
                if let Some(Some(sel)) = reselected(&names, gone, by, whole)
                    && !sel.is_empty()
                {
                    let range = sel_range(built.text(file), &idents);
                    let edit = Edit::new(j, range.start - h..range.end - h, self.selection(&sel));
                    if !out.iter().any(|e| e.same_place(&edit)) {
                        out.push(edit);
                    }
                }
            }
        }
        out
    }
}

/// A unique or index clause of a relation variable, as [`Model::scoped`] reads it.
struct Scoped {
    relation: u64,
    at: usize,
    kind: IndexKind,
    elems: Vec<String>,
    /// The constructors its scope reaches, every one when `None`.
    reached: Option<Vec<u64>>,
    rescoped: Option<Vec<Term>>,
}

/// What a routing gives: what it carries whole, its function of a row and its canonical form.
pub(super) struct Routing {
    carries: Carries,
    rows: RowFn,
    text: String,
    made: Vec<(u64, Option<Vec<u64>>)>,
}

impl Routing {
    /// Refuse as broken unless `made`, the products of `model`, have the ids of the products
    /// the routing was typed against.
    fn typed_for(&self, model: &Model, made: &[u64]) -> Res<()> {
        let held: Vec<(u64, Option<Vec<u64>>)> = made
            .iter()
            .map(|c| (*c, model.ctors[c].fields.clone()))
            .collect();
        if held == self.made {
            Ok(())
        } else {
            broken("a routing typed against products of other ids")
        }
    }
}
