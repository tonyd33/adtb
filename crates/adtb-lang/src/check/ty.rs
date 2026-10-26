use crate::schema::{Schema, TypeDefKind};
use crate::types::*;

/// Returns whether type parameter `i` of `def` occurs in a field, so is covariant; otherwise it is bivariant.
fn occurs_in_field(schema: &Schema, def: TypeId, i: u32) -> bool {
    let t = schema.ty(def);
    match &t.kind {
        TypeDefKind::Newtype { .. } => false,
        TypeDefKind::Sum { header, ctors, .. } => header
            .iter()
            .chain(ctors.iter().flat_map(|c| c.fields.iter()))
            .any(|f| mentions_param(&f.ty, i)),
    }
}

fn mentions_param(ty: &Ty, i: u32) -> bool {
    match ty {
        Ty::Param(j) => *j == i,
        Ty::Data { args, .. } => args.iter().any(|a| mentions_param(a, i)),
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => mentions_param(t, i),
        Ty::Group(a, b) | Ty::Fn(a, b) => mentions_param(a, i) || mentions_param(b, i),
        Ty::Record(fs) => fs.iter().any(|(_, t)| mentions_param(t, i)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, t)| mentions_param(t, i))),
        Ty::Base(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
    }
}

fn subset(a: &Option<CtorSet>, b: &Option<CtorSet>) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(a), Some(b)) => a.iter().all(|c| b.contains(c)),
    }
}

fn union(
    schema: &Schema,
    def: TypeId,
    a: &Option<CtorSet>,
    b: &Option<CtorSet>,
) -> Option<CtorSet> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let mut u: CtorSet = a.iter().chain(b.iter()).copied().collect();
            u.sort_unstable();
            u.dedup();
            schema.normalize_ctors(def, u)
        }
        _ => None,
    }
}

type Ctors = [(String, Vec<(String, Ty)>)];

/// Returns the names of the constructors `sel` selects, every one for `None`.
pub fn sum_selected<'c>(ctors: &'c Ctors, sel: &Option<CtorSet>) -> Vec<&'c str> {
    match sel {
        None => ctors.iter().map(|(c, _)| c.as_str()).collect(),
        Some(s) => s.iter().map(|&i| ctors[i as usize].0.as_str()).collect(),
    }
}

/// Returns the selection of `ctors` that names `names`, `None` when it names every constructor.
pub fn sum_sel(ctors: &Ctors, names: &[&str]) -> Option<CtorSet> {
    let mut set: CtorSet = ctors
        .iter()
        .enumerate()
        .filter(|(_, (c, _))| names.contains(&c.as_str()))
        .map(|(i, _)| i as u32)
        .collect();
    set.sort_unstable();
    (set.len() != ctors.len()).then_some(set)
}

fn equiv(schema: &Schema, a: &Ty, b: &Ty) -> bool {
    subtype(schema, a, b) && subtype(schema, b, a)
}

fn same_fields(schema: &Schema, a: &[(String, Ty)], b: &[(String, Ty)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(n, t)| b.iter().any(|(m, u)| n == m && equiv(schema, t, u)))
}

/// Returns whether two anonymous sums have the same constructors, in any order, with equivalent fields.
fn same_ctors(schema: &Schema, a: &Ctors, b: &Ctors) -> bool {
    a.len() == b.len()
        && a.iter().all(|(c, fs)| {
            b.iter()
                .any(|(d, gs)| c == d && same_fields(schema, fs, gs))
        })
}

/// `a <: b`.
pub fn subtype(schema: &Schema, a: &Ty, b: &Ty) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (
            Ty::Data {
                def: d1,
                args: a1,
                ctors: c1,
            },
            Ty::Data {
                def: d2,
                args: a2,
                ctors: c2,
            },
        ) => {
            d1 == d2
                && subset(c1, c2)
                && a1.iter().zip(a2).enumerate().all(|(i, (x, y))| {
                    !occurs_in_field(schema, *d1, i as u32) || subtype(schema, x, y)
                })
        }
        (Ty::Key { rel: r1, ctors: c1 }, Ty::Key { rel: r2, ctors: c2 }) => {
            r1 == r2 && subset(c1, c2)
        }
        (Ty::Set(x) | Ty::NonEmpty(x), Ty::Set(y)) | (Ty::NonEmpty(x), Ty::NonEmpty(y)) => {
            subtype(schema, x, y)
        }
        (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => subtype(schema, p2, p1) && subtype(schema, r1, r2),
        (Ty::Record(x), Ty::Record(y)) => same_fields(schema, x, y),
        (Ty::Tag(x), Ty::Tag(y)) => equiv(schema, x, y),
        (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 }) => {
            same_ctors(schema, x, y) && {
                let want = sum_selected(y, s2);
                sum_selected(x, s1).iter().all(|c| want.contains(c))
            }
        }
        _ => false,
    }
}

/// The least upper bound of `a` and `b`, where one exists.
pub fn join(schema: &Schema, a: &Ty, b: &Ty) -> Option<Ty> {
    if subtype(schema, a, b) {
        return Some(b.clone());
    }
    if subtype(schema, b, a) {
        return Some(a.clone());
    }
    match (a, b) {
        (
            Ty::Data {
                def: d1,
                args: a1,
                ctors: c1,
            },
            Ty::Data {
                def: d2,
                args: a2,
                ctors: c2,
            },
        ) if d1 == d2 => {
            let mut args = Vec::new();
            for (i, (x, y)) in a1.iter().zip(a2).enumerate() {
                if occurs_in_field(schema, *d1, i as u32) {
                    args.push(join(schema, x, y)?);
                } else {
                    args.push(x.clone());
                }
            }
            Some(Ty::Data {
                def: *d1,
                args,
                ctors: union(schema, *d1, c1, c2),
            })
        }
        (Ty::Key { rel: r1, ctors: c1 }, Ty::Key { rel: r2, ctors: c2 }) if r1 == r2 => {
            let def = schema.relation(*r1).elem_def;
            Some(Ty::Key {
                rel: *r1,
                ctors: union(schema, def, c1, c2),
            })
        }
        (Ty::NonEmpty(x), Ty::NonEmpty(y)) => Some(Ty::NonEmpty(Box::new(join(schema, x, y)?))),
        (Ty::Set(x) | Ty::NonEmpty(x), Ty::Set(y) | Ty::NonEmpty(y)) => {
            Some(Ty::Set(Box::new(join(schema, x, y)?)))
        }
        (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 })
            if same_ctors(schema, x, y) =>
        {
            let mut names = sum_selected(x, s1);
            names.extend(sum_selected(y, s2));
            Some(Ty::Sum {
                ctors: x.to_vec(),
                sel: sum_sel(x, &names),
            })
        }
        _ => None,
    }
}

/// Returns the join of `a` and `b` in ad hoc text, where two anonymous sums join at the union of their constructors.
pub fn join_adhoc(schema: &Schema, a: &Ty, b: &Ty) -> Option<Ty> {
    if let Some(j) = join(schema, a, b) {
        return Some(j);
    }
    match (a, b) {
        (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 }) => {
            let mut out = x.clone();
            for (c, fs) in y {
                match out.iter().find(|(d, _)| d == c) {
                    Some((_, gs)) if !same_fields(schema, gs, fs) => return None,
                    Some(_) => {}
                    None => out.push((c.clone(), fs.clone())),
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            let mut names = sum_selected(x, s1);
            names.extend(sum_selected(y, s2));
            let sel = sum_sel(&out, &names);
            Some(Ty::Sum { ctors: out, sel })
        }
        (Ty::Set(x) | Ty::NonEmpty(x), Ty::Set(y) | Ty::NonEmpty(y)) => {
            Some(Ty::Set(Box::new(join_adhoc(schema, x, y)?)))
        }
        _ => None,
    }
}

/// The element type of a set, a nonempty set or a group.
pub fn elem(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Group(_, t) => Some(t),
        _ => None,
    }
}

/// Every field a value of `ty` has, in field-access order: header fields, then the fields every constructor shares.
pub fn fields(schema: &Schema, ty: &Ty) -> Vec<(String, Ty)> {
    match ty {
        Ty::Record(fs) => fs.clone(),
        Ty::Sum { ctors, sel } => {
            let names = sum_selected(ctors, sel);
            let mut chosen = ctors.iter().filter(|(c, _)| names.contains(&c.as_str()));
            let Some((_, first)) = chosen.next() else {
                return Vec::new();
            };
            let rest: Vec<&Vec<(String, Ty)>> = chosen.map(|(_, fs)| fs).collect();
            first
                .iter()
                .filter(|(n, t)| {
                    rest.iter()
                        .all(|fs| fs.iter().any(|(m, u)| m == n && equiv(schema, t, u)))
                })
                .cloned()
                .collect()
        }
        Ty::Data { def, args, ctors } => {
            let t = schema.ty(*def);
            let TypeDefKind::Sum {
                header, ctors: all, ..
            } = &t.kind
            else {
                return Vec::new();
            };
            let mut out: Vec<(String, Ty)> = header
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(args)))
                .collect();
            let set: Vec<u32> = match ctors {
                Some(s) => s.clone(),
                None if all.len() == 1 => vec![0],
                None => return out,
            };
            let Some(&first) = set.first() else {
                return out;
            };
            for f in &all[first as usize].fields {
                let fty = f.ty.subst(args);
                let shared = set.iter().all(|&c| {
                    all[c as usize]
                        .fields
                        .iter()
                        .any(|g| g.name == f.name && g.ty.subst(args) == fty)
                });
                if shared {
                    out.push((f.name.clone(), fty));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The fields of constructor `c` of `def` at `args`, header fields first.
pub fn ctor_fields(schema: &Schema, def: TypeId, args: &[Ty], c: u32) -> Vec<(String, Ty, bool)> {
    let t = schema.ty(def);
    let mut out: Vec<(String, Ty, bool)> = t
        .header()
        .iter()
        .map(|f| (f.name.clone(), f.ty.subst(args), true))
        .collect();
    if let Some(ctor) = t.ctors().get(c as usize) {
        out.extend(
            ctor.fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(args), false)),
        );
    }
    out
}

/// The constructors of a sum type `ty`, by index, each with its name.
pub fn ctor_names(schema: &Schema, ty: &Ty) -> Option<Vec<(u32, String)>> {
    match ty {
        Ty::Data { def, ctors, .. } => {
            let t = schema.ty(*def);
            if !matches!(t.kind, TypeDefKind::Sum { .. }) {
                return None;
            }
            let all: Vec<u32> = (0..t.ctors().len() as u32).collect();
            let set = ctors.as_ref().unwrap_or(&all);
            Some(
                set.iter()
                    .map(|&c| (c, t.ctors()[c as usize].name.clone()))
                    .collect(),
            )
        }
        Ty::Tag(inner) => {
            let Ty::Data { def, .. } = inner.as_ref() else {
                return None;
            };
            let t = schema.ty(*def);
            Some(
                t.ctors()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i as u32, c.name.clone()))
                    .collect(),
            )
        }
        Ty::Sum { ctors, sel } => Some(
            ctors
                .iter()
                .enumerate()
                .filter(|(i, _)| sel.as_ref().is_none_or(|s| s.contains(&(*i as u32))))
                .map(|(i, (n, _))| (i as u32, n.clone()))
                .collect(),
        ),
        _ => None,
    }
}

/// `ty` narrowed to the constructors `set` of its sum.
pub fn restrict(schema: &Schema, ty: &Ty, set: CtorSet) -> Ty {
    match ty {
        Ty::Data { def, args, .. } => Ty::Data {
            def: *def,
            args: args.clone(),
            ctors: schema.normalize_ctors(*def, set),
        },
        Ty::Sum { ctors, .. } => Ty::Sum {
            ctors: ctors.clone(),
            sel: (set.len() != ctors.len()).then_some(set),
        },
        _ => ty.clone(),
    }
}

/// The fields of one constructor of a sum type at its arguments, header first.
pub fn fields_of_ctor(schema: &Schema, ty: &Ty, c: u32) -> Vec<(String, Ty, bool)> {
    match ty {
        Ty::Data { def, args, .. } => ctor_fields(schema, *def, args, c),
        Ty::Sum { ctors, .. } => ctors[c as usize]
            .1
            .iter()
            .map(|(n, t)| (n.clone(), t.clone(), false))
            .collect(),
        _ => Vec::new(),
    }
}
