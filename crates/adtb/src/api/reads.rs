//! What the callables of a bundle read and write, from their normal forms, as a reader of the
//! catalog states it.

use std::collections::BTreeSet;

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty, TypeId};

use crate::catalog::{Read, Reader};

/// The reads and writes of some normal forms over one schema.
#[derive(Debug, Default)]
pub(super) struct Reads {
    whole: BTreeSet<RelId>,
    references: BTreeSet<(RelId, Option<CtorSet>)>,
    /// The fields read by access, by the type of the row read.
    fields: BTreeSet<(TypeId, String)>,
    /// The types of the rows used whole: returned, compared or passed on.
    rows: BTreeSet<TypeId>,
    /// The relations whose rows are reached at all.
    reached: BTreeSet<RelId>,
    writes: BTreeSet<(RelId, u32)>,
}

impl Reads {
    /// Add what the normal form `root` of `terms` reads and writes.
    pub(super) fn add(&mut self, schema: &Schema, terms: &Terms, root: Tm) {
        let mut seen = BTreeSet::new();
        let mut stack = vec![(root, false)];
        while let Some((t, transparent)) = stack.pop() {
            if !seen.insert((t, transparent)) {
                continue;
            }
            if !transparent {
                rows_in(terms.ty(t), &mut self.rows);
            }
            self.facts(schema, terms, t);
            for (c, through) in children(terms, t) {
                stack.push((c, through));
            }
        }
    }

    /// Record the reads and writes `t` makes itself.
    fn facts(&mut self, schema: &Schema, terms: &Terms, t: Tm) {
        match terms.kind(t) {
            Kind::Rel(r, set) | Kind::Transition(_, r, set) => {
                self.reached.insert(*r);
                if set.as_ref().is_none_or(|s| covers(schema, *r, s)) {
                    self.whole.insert(*r);
                }
            }
            Kind::Deref { key, .. } => {
                if let Ty::Key { rel, ctors } = terms.ty(*key) {
                    self.reached.insert(*rel);
                    let ctors = ctors.clone().filter(|s| !covers(schema, *rel, s));
                    self.references.insert((*rel, ctors));
                }
            }
            Kind::Field(x, name) => {
                if let Ty::Data { def, .. } = terms.ty(*x) {
                    self.fields.insert((*def, name.clone()));
                }
            }
            Kind::Insert(r, rows) | Kind::Upsert { rel: r, rows, .. } => {
                self.written(schema, *r, element(terms.ty(*rows)));
            }
            Kind::Replace(r, _, f) => {
                let out = match terms.ty(*f) {
                    Ty::Fn(_, out) => Some(&**out),
                    _ => None,
                };
                self.written(schema, *r, out);
            }
            Kind::Rejects { into, .. } => self.written(schema, *into, None),
            _ => {}
        }
    }

    /// Record a write into `r` of rows of type `row`: into its constructors, else into every
    /// constructor of `r`.
    fn written(&mut self, schema: &Schema, r: RelId, row: Option<&Ty>) {
        let ctors = match row {
            Some(Ty::Data {
                ctors: Some(cs), ..
            }) => cs.clone(),
            _ => (0..ctor_count(schema, r)).collect(),
        };
        self.writes.extend(ctors.into_iter().map(|c| (r, c)));
    }

    /// Returns the reader of `bundle`, pinned at `version` with `schema` its schema, that reads
    /// and writes what these reads do.
    pub(super) fn reader(
        &self,
        schema: &Schema,
        bundle: &str,
        version: u64,
        hides_new: bool,
    ) -> Reader {
        let qualified = |r: RelId| {
            let rel = schema.relation(r);
            format!("{}::{}", schema.module(rel.module).name, rel.name)
        };
        let mut reads: Vec<Read> = self
            .whole
            .iter()
            .map(|&r| Read::Relation(qualified(r)))
            .collect();
        for (r, ctors) in &self.references {
            let set = ctors.as_ref().and_then(|cs| {
                schema
                    .relation(*r)
                    .sets
                    .iter()
                    .find(|s| same(&s.ctors, cs))
                    .map(|s| s.name.clone())
            });
            reads.push(Read::Reference {
                relation: qualified(*r),
                set,
            });
        }
        for &r in &self.reached {
            let def = schema.relation(r).elem_def;
            let names: BTreeSet<&str> = if self.rows.contains(&def) {
                field_names(schema, def)
            } else {
                self.fields
                    .iter()
                    .filter(|(d, n)| *d == def && field_names(schema, def).contains(n.as_str()))
                    .map(|(_, n)| n.as_str())
                    .collect()
            };
            reads.extend(names.into_iter().map(|n| Read::Field {
                relation: qualified(r),
                field: n.to_owned(),
            }));
        }
        let writes = self
            .writes
            .iter()
            .map(|&(r, c)| {
                let def = schema.relation(r).elem_def;
                (
                    qualified(r),
                    schema.ty(def).ctors()[c as usize].name.clone(),
                )
            })
            .collect();
        Reader {
            bundle: bundle.to_owned(),
            version,
            hides_new,
            reads,
            writes,
        }
    }
}

/// Returns each child of `t` with whether its position passes a row on without reading it: a
/// field access, a key or tag, a binding whose uses are visited, a source ranged over, a branch
/// whose value is the parent's, or rows a write copies.
fn children(terms: &Terms, t: Tm) -> Vec<(Tm, bool)> {
    let through = |ts: &[Tm]| ts.iter().map(|&c| (c, true)).collect::<Vec<_>>();
    match terms.kind(t) {
        Kind::Field(x, _)
        | Kind::Key(x)
        | Kind::KeyOf(_, x)
        | Kind::Tag(x)
        | Kind::Restrict(x, _)
        | Kind::Only(x)
        | Kind::IsEmpty(x)
        | Kind::Exists(x)
        | Kind::Singleton(x)
        | Kind::Lam(_, x)
        | Kind::Insert(_, x)
        | Kind::Delete(_, x) => through(&[*x]),
        Kind::If(c, a, b) => vec![(*c, false), (*a, true), (*b, true)],
        Kind::Let(_, e, body) | Kind::For(_, e, body) | Kind::LetBang(_, e, body) => {
            through(&[*e, *body])
        }
        Kind::Union(a, b) | Kind::Replace(_, a, b) => through(&[*a, *b]),
        Kind::Case { scrutinee, arms } => {
            let mut out = vec![(*scrutinee, arms_keep_rows(terms, *scrutinee, arms))];
            out.extend(arms.iter().map(|&(_, _, b)| (b, true)));
            out
        }
        Kind::Agg { source, body, .. } => vec![(*source, true), (*body, false)],
        Kind::Group { source, by, .. } => vec![(*source, true), (*by, false)],
        Kind::Top {
            count,
            keys,
            source,
            ..
        } => {
            let mut out = vec![(*count, false), (*source, true)];
            out.extend(keys.iter().map(|&(k, _)| (k, false)));
            out
        }
        Kind::Present {
            source,
            keys,
            take,
            after,
            ..
        } => {
            let mut out = vec![(*source, true)];
            out.extend(keys.iter().map(|&(k, _)| (k, false)));
            out.extend(take.iter().chain(after).map(|&c| (c, false)));
            out
        }
        Kind::Fix(binds, body) => {
            let mut out: Vec<(Tm, bool)> = binds.iter().map(|&(_, e)| (e, true)).collect();
            out.push((*body, true));
            out
        }
        Kind::At { point, body, .. } => vec![(*point, false), (*body, true)],
        Kind::Upsert {
            rows, otherwise, ..
        } => through(&[*rows, *otherwise]),
        Kind::Rejects { insert, .. } => through(&[*insert]),
        Kind::Par(ws) => through(ws),
        _ => terms.children(t).into_iter().map(|c| (c, false)).collect(),
    }
}

/// Returns whether every arm of a `match` on `scrutinee` binds a row of the scrutinee's type, so
/// the arms' uses of their variables are what reads it.
fn arms_keep_rows(
    terms: &Terms,
    scrutinee: Tm,
    arms: &[(u32, adtb_lang::kernel::Var, Tm)],
) -> bool {
    let Ty::Data { def, .. } = terms.ty(scrutinee) else {
        return true;
    };
    arms.iter()
        .all(|&(_, v, _)| matches!(&terms.var_info(v).ty, Ty::Data { def: d, .. } if d == def))
}

/// Add every declared type a value of `ty` holds at any depth, through no reference.
fn rows_in(ty: &Ty, out: &mut BTreeSet<TypeId>) {
    match ty {
        Ty::Data { def, args, .. } => {
            out.insert(*def);
            for a in args {
                rows_in(a, out);
            }
        }
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => rows_in(t, out),
        Ty::Group(k, v) | Ty::Fn(k, v) => {
            rows_in(k, out);
            rows_in(v, out);
        }
        Ty::Record(fs) => fs.iter().for_each(|(_, t)| rows_in(t, out)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .flat_map(|(_, fs)| fs)
            .for_each(|(_, t)| rows_in(t, out)),
        Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => {}
    }
}

/// Returns the element type of a set type.
fn element(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Set(t) | Ty::NonEmpty(t) => Some(t),
        _ => None,
    }
}

fn ctor_count(schema: &Schema, r: RelId) -> u32 {
    let n = schema.ty(schema.relation(r).elem_def).ctors().len();
    u32::try_from(n).expect("a sum has fewer than 2^32 constructors")
}

/// Returns whether `set` holds every constructor of `r`.
fn covers(schema: &Schema, r: RelId, set: &CtorSet) -> bool {
    (0..ctor_count(schema, r)).all(|c| set.contains(&c))
}

fn same(a: &[u32], b: &[u32]) -> bool {
    let a: BTreeSet<&u32> = a.iter().collect();
    let b: BTreeSet<&u32> = b.iter().collect();
    a == b
}

/// Returns the name of every field of the rows of type `def`, header and constructors.
fn field_names(schema: &Schema, def: TypeId) -> BTreeSet<&str> {
    let t = schema.ty(def);
    t.header()
        .iter()
        .chain(t.ctors().iter().flat_map(|c| &c.fields))
        .map(|f| f.name.as_str())
        .collect()
}
