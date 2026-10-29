//! Values of a bundle pinned before the current version: its arguments carried forward to the
//! current version, its results encoded at the pinned one.

use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::translate::Map;
use adtb_lang::types::Ty;

use crate::catalog::layout::repr;
use crate::exec::Val;

/// Returns `v`, a value of `ty` at the pinned version, as the current version holds it.
///
/// Preconditions:
/// - `ty` carries whole and exactly to the current version: every constructor and field it
///   reaches has a counterpart, and it reaches no field the current version adds.
pub(super) fn forward(map: &Map<'_>, ty: &Ty, v: &Val) -> Val {
    recode(map, ty, v, true)
}

/// Returns `v`, a value of the current version's image of `ty`, as the pinned version holds it.
///
/// Preconditions:
/// - Every field of the pinned version that `ty` reaches has a counterpart.
/// - `v` holds no constructor added since the pinned version.
pub(super) fn back(map: &Map<'_>, ty: &Ty, v: &Val) -> Val {
    recode(map, ty, v, false)
}

fn recode(map: &Map<'_>, ty: &Ty, v: &Val, forward: bool) -> Val {
    let ty = repr(map.from, ty);
    match (&ty, v) {
        (Ty::Data { def, args, .. }, Val::Ctor(c, fs)) => {
            let (pin, now) = if forward {
                (Some(*c), map.ctor(*def, *c))
            } else {
                (map.ctor_back(*def, *c), Some(*c))
            };
            let (Some(pin), Some(now), Some(to)) = (pin, now, map.def(*def)) else {
                panic!("a carried value's constructor has a counterpart at both versions");
            };
            let to_args = args
                .iter()
                .map(|a| map.ty(a))
                .collect::<Result<Vec<_>, _>>()
                .expect("a carried value's type has a counterpart at both versions");
            let pinned = Map::fields(map.from, *def, args, pin);
            let current = Map::fields(map.to, to, &to_args, now);
            let image = |f: &str| map.field(*def, Some(pin), f);
            let fields: Vec<Val> = if forward {
                current
                    .iter()
                    .map(|(n, _)| {
                        let at = pinned
                            .iter()
                            .position(|(f, _)| image(f).as_deref() == Some(n))
                            .expect("a whole value's every current field has a pinned one");
                        recode(map, &pinned[at].1, &fs[at], true)
                    })
                    .collect()
            } else {
                pinned
                    .iter()
                    .map(|(f, t)| {
                        let n =
                            image(f).expect("a returned value's every pinned field has an image");
                        let at = current
                            .iter()
                            .position(|(c, _)| *c == n)
                            .expect("an image field is a field of the current constructor");
                        recode(map, t, &fs[at], false)
                    })
                    .collect()
            };
            Val::ctor(if forward { now } else { pin }, fields)
        }
        (Ty::Tag(t), Val::Ctor(c, fs)) if fs.is_empty() => {
            let Ty::Data { def, .. } = repr(map.from, t) else {
                return v.clone();
            };
            let c = if forward {
                map.ctor(def, *c)
            } else {
                map.ctor_back(def, *c)
            };
            Val::tag(c.expect("a carried tag's constructor has a counterpart at both versions"))
        }
        (Ty::Record(ts), Val::Record(fs)) => Val::record(
            ts.iter()
                .zip(fs.iter())
                .map(|((_, t), f)| recode(map, t, f, forward))
                .collect(),
        ),
        (Ty::Sum { ctors, .. }, Val::Ctor(c, fs)) => {
            let Some((_, ts)) = ctors.get(*c as usize) else {
                return v.clone();
            };
            let fields = ts
                .iter()
                .zip(fs.iter())
                .map(|((_, t), f)| recode(map, t, f, forward))
                .collect();
            Val::ctor(*c, fields)
        }
        (Ty::Set(t) | Ty::NonEmpty(t), Val::Set(s)) => Val::set(
            s.iter()
                .map(|e| recode(map, t, e, forward))
                .collect::<BTreeSet<_>>(),
        ),
        (Ty::Set(t) | Ty::NonEmpty(t), Val::List(l)) => {
            let mut seen = BTreeSet::new();
            let elems: Vec<Val> = l
                .iter()
                .map(|e| recode(map, t, e, forward))
                .filter(|e| seen.insert(e.clone()))
                .collect();
            Val::List(Arc::from(elems))
        }
        _ => v.clone(),
    }
}
