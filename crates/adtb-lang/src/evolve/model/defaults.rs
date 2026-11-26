use std::ops::Range;

use super::clause::after_eq;
use super::field::Applied;
use super::op::{Res, broken, refuse, refuse_at};
use super::print::Printed;
use super::run::{Built, Late, Run};
use super::*;
use crate::ast::{self, RelationClause};
use crate::canon::canonical_term;
use crate::evolve::ctor::Carries;
use crate::evolve::{Clause, Resolved};
use crate::span::FileId;

/// The defaults of a relation variable, one for each field it fills, by the field's id.
#[derive(Clone, Debug, Default)]
pub(super) struct Defaults(BTreeMap<u64, Held>);

/// The default of one field: its id and its value.
#[derive(Clone, Debug)]
struct Held {
    id: u64,
    value: Term,
}

/// A `default` clause as printed: the fields it fills, in declaration order, and the constructor
/// it names, `None` when it names none.
struct Group {
    fields: Vec<u64>,
    ctor: Option<u64>,
}

/// Why a default on a field a structural operator replaces has no image.
enum Unimaged {
    /// The routing carries the field whole to no product field.
    Uncarried,
    /// The field has no other image.
    Unlike,
}

/// How a structural operator takes the fields of the constructors it replaces to those it makes.
#[derive(Clone, Copy)]
pub(super) enum Taken<'a> {
    /// Each product takes the default of each field the routing takes to one of its fields.
    Split(&'a Carries),
    /// A product field takes a default when every source has one with one value on the field
    /// the routing carries to it.
    Merge(&'a Carries),
    /// Each product holding a field of the source field's name takes its default.
    Unnest,
    /// No default on a source has an image.
    Nest,
}

impl Defaults {
    /// Returns the defaults `fills` lists, each a field, its default's id and its value.
    ///
    /// Fail when a field is listed twice.
    pub(super) fn lifted(fills: Vec<(u64, u64, Term)>) -> Result<Defaults, String> {
        let mut out = BTreeMap::new();
        for (field, id, value) in fills {
            if out.insert(field, Held { id, value }).is_some() {
                return Err(format!("field {field} has two defaults"));
            }
        }
        Ok(Defaults(out))
    }

    /// Returns whether default `id` is one of these.
    pub(super) fn holds(&self, id: u64) -> bool {
        self.0.values().any(|h| h.id == id)
    }

    /// Returns the id of the default of field `f`.
    pub(super) fn id_of(&self, f: u64) -> Option<u64> {
        self.0.get(&f).map(|h| h.id)
    }

    /// Returns the value of each default.
    pub(super) fn values(&self) -> impl Iterator<Item = &Term> {
        self.0.values().map(|h| &h.value)
    }

    /// Call `f` on the value of each default.
    pub(super) fn each_value_mut(&mut self, f: &mut dyn FnMut(&mut Term)) {
        for h in self.0.values_mut() {
            f(&mut h.value);
        }
    }
}

impl Model {
    /// Returns the `default` clauses of relation variable `r` as printed, in declaration order of
    /// their first fields: `default f` for the header field `f`, for the field `f` of a type's
    /// only constructor, and when two or more constructors have a field `f` and each has a
    /// default of one value; else `default C.f` for each field.
    fn default_groups(&self, r: u64) -> Vec<Group> {
        let rel = &self.relations[&r];
        let Some(t) = self.relation_type(r) else {
            return Vec::new();
        };
        let printed = |f: &u64| rel.defaults.0.get(f).map(|h| h.value.print(self));
        let mut out = Vec::new();
        let mut done = BTreeSet::new();
        for f in self.fields_of(t) {
            let Some(value) = printed(&f).filter(|_| !done.contains(&f)) else {
                continue;
            };
            let owner = self.fields[&f].owner;
            if owner == t {
                done.insert(f);
                out.push(Group {
                    fields: vec![f],
                    ctor: None,
                });
                continue;
            }
            let name = &self.fields[&f].name;
            let ctors = self.ctors_of(t);
            let peers: Vec<u64> = ctors
                .iter()
                .filter_map(|&c| self.field_of(t, Some(c), name))
                .collect();
            let shared = peers.len() > 1 || ctors.len() == 1;
            let (fields, ctor) =
                if shared && peers.iter().all(|p| printed(p).as_ref() == Some(&value)) {
                    (peers, None)
                } else {
                    (vec![f], Some(owner))
                };
            done.extend(fields.iter().copied());
            out.push(Group { fields, ctor });
        }
        out
    }

    /// Print the `default` clauses of relation variable `r`, each on a line of its own.
    pub(super) fn print_defaults(&self, r: u64, out: &mut Printed) {
        let rel = &self.relations[&r];
        for g in self.default_groups(r) {
            let first = g.fields[0];
            out.push("\n  default ");
            if let Some(c) = g.ctor {
                out.push(&self.ctors[&c].name);
                out.push(".");
            }
            out.push(&self.fields[&first].name);
            out.push(" = ");
            out.term(&rel.defaults.0[&first].value, self);
            out.alias(g.fields.len() - 1);
        }
    }

    /// Returns the fields each `default` clause of relation variable `r` fills, in the order
    /// [`Model::print_defaults`] prints the clauses.
    pub(super) fn default_clause_fields(&self, r: u64) -> Vec<Vec<u64>> {
        self.default_groups(r)
            .into_iter()
            .map(|g| g.fields)
            .collect()
    }

    /// Returns the fields each `default` clause of `decl`, a declaration of relation variable
    /// `r`, fills, in clause order, none for a clause naming no field `r` has.
    pub(super) fn written_default_fields(&self, r: u64, decl: &ast::RelationDecl) -> Vec<Vec<u64>> {
        decl.clauses
            .iter()
            .filter_map(|c| match c {
                RelationClause::Default { ctor, field, .. } => Some(
                    self.default_targets(r, ctor.as_ref(), &field.text)
                        .unwrap_or_default(),
                ),
                _ => None,
            })
            .collect()
    }

    /// Returns the fields of relation variable `r` that have a default, in the order
    /// [`Model::print_defaults`] prints them.
    pub(super) fn default_order(&self, r: u64) -> Vec<u64> {
        self.default_groups(r)
            .into_iter()
            .flat_map(|g| g.fields)
            .collect()
    }

    /// Replace bytes `range` of the value of the default of field `f` of relation variable `r`,
    /// as `names` prints it, by `with`.
    ///
    /// Fail when `f` has no default in `r` or an end of `range` falls inside a name.
    pub(super) fn edit_default(
        &mut self,
        names: &Model,
        (r, f): (u64, u64),
        range: Range<usize>,
        with: Term,
    ) -> Result<(), String> {
        let rel = self.relations.get_mut(&r).expect("a relation variable");
        let held = rel
            .defaults
            .0
            .get_mut(&f)
            .ok_or("an edit of a default the relation variable does not hold")?;
        held.value.replace(names, range, with)
    }

    /// Returns the position among the clauses relation variable `r` prints of the `default`
    /// clause that prints default `default`.
    pub(super) fn default_place(&self, r: u64, default: u64) -> Option<usize> {
        let rel = self.relations.get(&r)?;
        let g = self.default_groups(r).iter().position(|g| {
            g.fields
                .iter()
                .any(|f| rel.defaults.0.get(f).is_some_and(|h| h.id == default))
        })?;
        Some(rel.clauses.len() + g)
    }

    /// Returns the default of field `f` of relation variable `r` as a refusal names it: `r.C.f`,
    /// or `r.f` for a header field.
    fn default_shown(&self, r: u64, f: u64) -> String {
        let field = &self.fields[&f];
        match self.ctors.get(&field.owner) {
            Some(c) => format!("{}.{}.{}", self.relations[&r].name, c.name, field.name),
            None => format!("{}.{}", self.relations[&r].name, field.name),
        }
    }

    /// Returns a `Member` clause for each constructor whose field of `fields` a default fills.
    fn default_members(&self, fields: &[u64]) -> Vec<Clause> {
        fields
            .iter()
            .filter_map(|f| {
                let owner = self.fields.get(f)?.owner;
                self.ctors
                    .contains_key(&owner)
                    .then_some(Clause::Member { ctor: owner })
            })
            .collect()
    }

    /// Returns the fields a default written on `field` of relation variable `r` fills: the header
    /// field of that name, else that field of `ctor`, or of each constructor declaring it when
    /// `ctor` is `None`, in declaration order.
    ///
    /// Refuse when `ctor` names no constructor of the relation variable's type.
    fn default_targets(&self, r: u64, ctor: Option<&ast::Ident>, field: &str) -> Res<Vec<u64>> {
        let Some(t) = self.relation_type(r) else {
            return broken("a relation variable of no type");
        };
        let ctors = self.ctors_of(t);
        if let Some(c) = ctor
            && !ctors.iter().any(|k| self.ctors[k].name == c.text)
        {
            return refuse_at(
                c.span,
                format!(
                    "`{}` is not a constructor of `{}`",
                    c.text, self.types[&t].name
                ),
            );
        }
        Ok(match self.field_of(t, None, field) {
            Some(f) => vec![f],
            None => ctors
                .into_iter()
                .filter(|c| ctor.is_none_or(|n| self.ctors[c].name == n.text))
                .filter_map(|c| self.field_of(t, Some(c), field))
                .collect(),
        })
    }

    pub(super) fn add_default(
        &mut self,
        run: &mut Run,
        r: u64,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
        value: Term,
    ) -> Res<Applied> {
        let fields = self.default_targets(r, ctor, &field.text)?;
        let Some(&first) = fields.first() else {
            let t = self.relation_type(r).expect("a typed relation variable");
            return refuse_at(
                field.span,
                format!("`{}` has no field `{}`", self.types[&t].name, field.text),
            );
        };
        if let Some(&f) = fields
            .iter()
            .find(|f| self.relations[&r].defaults.0.contains_key(f))
        {
            return refuse_at(
                field.span,
                format!("`{}` already has a default", self.default_shown(r, f)),
            );
        }
        let mut default = 0;
        for &f in &fields {
            let id = self.fresh(Space::Default);
            if f == first {
                default = id;
            }
            let held = Held {
                id,
                value: value.clone(),
            };
            let rel = self
                .relations
                .get_mut(&r)
                .expect("a resolved relation variable");
            rel.defaults.0.insert(f, held);
        }
        let members = self.default_members(&fields);
        let resolved = |value| Resolved::AddDefault {
            default,
            relation: r,
            field: first,
            value,
        };
        let Some((built, file)) = self.rebuilt_relation(run, r)? else {
            let Some(k) = self.default_place(r, default) else {
                return broken("an added default the model does not print");
            };
            self.clause_typed(run, r, k, None)?;
            self.unbuilt_clauses.insert((Space::Default, default));
            let value = run.late(Late::Default {
                relation: r,
                default,
            });
            return Ok((resolved(value), members, Ok(Vec::new())));
        };
        let Some(k) = self.default_place(r, default) else {
            return broken("an added default its build does not hold");
        };
        self.clause_typed(run, r, k, Some(&built))?;
        let value = self.default_written(&built, file, k).unwrap_or_else(|| {
            run.late(Late::Default {
                relation: r,
                default,
            })
        });
        Ok((resolved(value), members, Ok(Vec::new())))
    }

    /// Drop the defaults `drop default` names: of `ctor`'s field `field`, or of every field named
    /// `field` of relation variable `r` without `ctor`.
    ///
    /// Refuse when none of them has a default.
    pub(super) fn drop_default(
        &mut self,
        run: &mut Run,
        r: u64,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> Res<Applied> {
        let named = self.default_targets(r, ctor, &field.text)?;
        let rel = &self.relations[&r];
        let dropped: Vec<u64> = named
            .into_iter()
            .filter(|f| rel.defaults.0.contains_key(f))
            .collect();
        let Some(first) = dropped.first() else {
            let target = match ctor {
                Some(c) => format!("{}.{}", c.text, field.text),
                None => field.text.clone(),
            };
            return refuse_at(
                field.span,
                format!("`{}` has no default for `{target}`", rel.name),
            );
        };
        let default = rel.defaults.0[first].id;
        let members = self.default_members(&dropped);
        let rel = self.relations.get_mut(&r).expect("the relation variable");
        for f in &dropped {
            rel.defaults.0.remove(f);
        }
        self.settle_ids(run);
        Ok((Resolved::DropDefault { default }, members, Ok(Vec::new())))
    }

    /// Returns the canonical form of the value of `default` clause `k` of the relation
    /// declaration of `file` in `built`, `None` when it does not resolve there.
    fn default_written(&self, built: &Built, file: FileId, k: usize) -> Option<String> {
        let Ok(RelationClause::Default { field, value, .. }) = Self::clause_written(built, file, k)
        else {
            return None;
        };
        let end = match &value {
            ast::DefaultValue::Generate(g) => g.span.end,
            ast::DefaultValue::Expr(e) => e.span.end,
        };
        let span = after_eq(built.text(file), field.span.end as usize, end as usize);
        canonical_term(built.text(file), span, &built.names(file)).ok()
    }

    /// Returns the canonical form of the value of default `default` of relation variable `r` as
    /// `built` writes it, `None` when it does not resolve there.
    pub(super) fn default_canonical(&self, built: &Built, r: u64, default: u64) -> Option<String> {
        let file = FileId(self.decl_at(Item::Relation(r))? as u32);
        self.default_written(built, file, self.default_place(r, default)?)
    }

    /// Returns the relation variable holding default `default`.
    pub(super) fn default_holder(&self, default: u64) -> Option<u64> {
        self.relations
            .iter()
            .find(|(_, rel)| rel.defaults.holds(default))
            .map(|(&r, _)| r)
    }

    /// Drop the default of each field of `fields` in every relation variable.
    pub(super) fn forget_defaults(&mut self, fields: &[u64]) {
        for rel in self.relations.values_mut() {
            rel.defaults.0.retain(|f, _| !fields.contains(f));
        }
    }

    /// Give each field of `to` a default of its own with the value of field `from`'s, in each
    /// relation variable where `from` has one.
    pub(super) fn copy_default(&mut self, from: u64, to: &[u64]) {
        let rels: Vec<u64> = self
            .relations
            .iter()
            .filter(|(_, rel)| rel.defaults.0.contains_key(&from))
            .map(|(&r, _)| r)
            .collect();
        for r in rels {
            for &f in to {
                let id = self.fresh(Space::Default);
                let rel = self.relations.get_mut(&r).expect("a relation variable");
                let value = rel.defaults.0[&from].value.clone();
                rel.defaults.0.insert(f, Held { id, value });
            }
        }
    }

    /// Returns the default of each field of `fields` in each relation variable, by the relation
    /// variable and the field, with its value, in the relation variables' and the fields' order.
    pub(super) fn defaults_of(&self, fields: &[u64]) -> Vec<(u64, u64, Term)> {
        let mut out = Vec::new();
        for (&r, rel) in &self.relations {
            for &f in fields {
                if let Some(h) = rel.defaults.0.get(&f) {
                    out.push((r, f, h.value.clone()));
                }
            }
        }
        out
    }

    /// Put each default of a relation variable of `t` on a field of a constructor of `sources` on
    /// the fields of `made` that `taken` takes its field to, each with a new default id. Refuse as
    /// the operator `what`, naming the default, one with no such image.
    pub(super) fn defaults_image(
        &mut self,
        built: &Built,
        t: u64,
        (sources, made): (&[u64], &[u64]),
        what: &str,
        taken: Taken<'_>,
    ) -> Res<()> {
        let own: Vec<u64> = sources
            .iter()
            .flat_map(|s| self.ctors[s].fields.clone().unwrap_or_default())
            .collect();
        for (r, ..) in self.rels_of(built, t) {
            let on: Vec<u64> = self
                .default_order(r)
                .into_iter()
                .filter(|f| own.contains(f))
                .collect();
            let mut put: BTreeMap<u64, Term> = BTreeMap::new();
            for &f in &on {
                let value = self.relations[&r].defaults.0[&f].value.clone();
                let images = match self.default_images(r, f, sources, made, taken) {
                    Ok(images) => images,
                    Err(Unimaged::Uncarried) => {
                        return refuse(format!(
                            "{what} is refused: `{}` has the default `{}` and its routing does \
                             not carry that field whole; drop the default first",
                            self.default_shown(r, f),
                            value.print(self)
                        ));
                    }
                    Err(Unimaged::Unlike) => {
                        return refuse(format!(
                            "{what} is refused: the default of `{}` has no image under it",
                            self.default_shown(r, f)
                        ));
                    }
                };
                for g in images {
                    put.entry(g).or_insert_with(|| value.clone());
                }
            }
            for (g, value) in put {
                let id = self.fresh(Space::Default);
                let rel = self.relations.get_mut(&r).expect("a relation variable");
                rel.defaults.0.insert(g, Held { id, value });
            }
        }
        Ok(())
    }

    /// Returns the fields of `made` that `taken` takes field `f` of a constructor of `sources`
    /// to, its default as relation variable `r` holds it taken there. A split or merge takes a
    /// field only to the product fields its routing carries it to whole.
    ///
    /// Fail with `Uncarried` when a split or merge carries `f` whole to no product field, and
    /// with `Unlike` when it has no other image: under a merge, a source lacks a default of the
    /// same value on the field the routing carries to the image.
    fn default_images(
        &self,
        r: u64,
        f: u64,
        sources: &[u64],
        made: &[u64],
        taken: Taken<'_>,
    ) -> Result<Vec<u64>, Unimaged> {
        let t = self.ctors[&made[0]].ty;
        let field = &self.fields[&f];
        let carried = |carries: &Carries, x: &FieldM, p: u64| -> Option<u64> {
            let (source, product) = (&self.ctors[&x.owner].name, &self.ctors[&p].name);
            let (.., g) = carries
                .iter()
                .find(|(s, y, z, _)| s == source && *y == x.name && z == product)?;
            self.field_of(t, Some(p), g)
        };
        let images: Vec<u64> = match taken {
            Taken::Nest => Vec::new(),
            Taken::Unnest => made
                .iter()
                .filter_map(|&p| self.field_of(t, Some(p), &field.name))
                .collect(),
            Taken::Split(carries) => {
                let images: Vec<u64> = made
                    .iter()
                    .filter_map(|&p| carried(carries, field, p))
                    .collect();
                if images.is_empty() {
                    return Err(Unimaged::Uncarried);
                }
                images
            }
            Taken::Merge(carries) => {
                let Some(g) = carried(carries, field, made[0]) else {
                    return Err(Unimaged::Uncarried);
                };
                let rel = &self.relations[&r];
                let value = rel.defaults.0[&f].value.print(self);
                let alike = sources.iter().all(|&s| {
                    self.ctors[&s].fields.iter().flatten().any(|x| {
                        carried(carries, &self.fields[x], made[0]) == Some(g)
                            && rel.defaults.0.get(x).map(|h| h.value.print(self))
                                == Some(value.clone())
                    })
                });
                if alike { vec![g] } else { Vec::new() }
            }
        };
        if images.is_empty() {
            return Err(Unimaged::Unlike);
        }
        Ok(images)
    }
}
