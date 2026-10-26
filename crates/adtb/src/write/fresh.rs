use std::collections::{BTreeMap, HashMap};

use adtb_lang::kernel::{Const, Kind, Terms, Tm, Var};
use adtb_lang::schema::{Generator, Schema};
use adtb_lang::types::{Base, RelId, Ty};

use crate::exec::Val;
use crate::exec::types::{ctors, repr};

/// The generator and relation of each `Fresh` occurrence, by its placeholder's number.
pub type Occurrences = BTreeMap<i64, (Generator, RelId)>;

/// Returns the variables `kind` binds around each of its children, in the order of
/// `Terms::children`.
fn bound(kind: &Kind, n: usize) -> Vec<Vec<Var>> {
    let mut out = vec![Vec::new(); n];
    match kind {
        Kind::Let(x, ..) | Kind::LetBang(x, ..) | Kind::For(x, ..) => out[1].push(*x),
        Kind::Lam(x, _) => out[0].push(*x),
        Kind::Upsert { old, new, .. } => out[1].extend([*old, *new]),
        Kind::Case { arms, .. } => {
            for (i, (_, v, _)) in arms.iter().enumerate() {
                out[i + 1].push(*v);
            }
        }
        Kind::Agg { var, .. } | Kind::Group { var, .. } => out[1].push(*var),
        Kind::Top { var, .. } => {
            for o in out.iter_mut().skip(2) {
                o.push(*var);
            }
        }
        Kind::Present { var, keys, .. } => {
            for o in out.iter_mut().skip(1).take(keys.len()) {
                o.push(*var);
            }
        }
        Kind::Fix(binds, _) => {
            let xs: Vec<Var> = binds.iter().map(|(x, _)| *x).collect();
            for o in &mut out {
                o.extend(&xs);
            }
        }
        _ => {}
    }
    out
}

/// Returns `t` with each path to a `Fresh` given its own placeholder: a record of its number
/// and the variables bound around it, so that its value is one per evaluation. Adds the
/// placeholders' numbers to `out`. `outer` holds the variables bound around `t`.
pub fn rewrite(terms: &mut Terms, t: Tm, outer: &[Var], out: &mut Occurrences) -> Tm {
    let mut fresh = HashMap::new();
    let mut scope = outer.to_vec();
    walk(terms, t, &mut scope, &mut fresh, out)
}

fn holds_fresh(terms: &Terms, t: Tm, memo: &mut HashMap<Tm, bool>) -> bool {
    if let Some(&h) = memo.get(&t) {
        return h;
    }
    let h = matches!(terms.kind(t), Kind::Fresh(..))
        || terms
            .children(t)
            .into_iter()
            .any(|c| holds_fresh(terms, c, memo));
    memo.insert(t, h);
    h
}

fn walk(
    terms: &mut Terms,
    t: Tm,
    scope: &mut Vec<Var>,
    memo: &mut HashMap<Tm, bool>,
    out: &mut Occurrences,
) -> Tm {
    if !holds_fresh(terms, t, memo) {
        return t;
    }
    if let Kind::Fresh(g, r) = terms.kind(t) {
        let (g, r, ty) = (*g, *r, terms.ty(t).clone());
        let occ = i64::try_from(terms.nodes.len()).expect("an arena of fewer than 2^63 nodes");
        let mut fields = vec![(
            "0000".to_string(),
            terms.add(
                Kind::Const(Const::Int(i128::from(occ))),
                Ty::Base(Base::I64),
            ),
        )];
        for (i, v) in scope.clone().into_iter().enumerate() {
            let vty = terms.var_info(v).ty.clone();
            fields.push((format!("{:04}", i + 1), terms.add(Kind::Var(v), vty)));
        }
        let p = terms.add(Kind::Record(fields), ty);
        out.insert(occ, (g, r));
        return p;
    }
    let kind = terms.kind(t).clone();
    let ty = terms.ty(t).clone();
    let cs = terms.children(t);
    let mut new = Vec::with_capacity(cs.len());
    for (c, b) in cs.iter().zip(bound(&kind, cs.len())) {
        let n = scope.len();
        scope.extend(b);
        new.push(walk(terms, *c, scope, memo, out));
        scope.truncate(n);
    }
    terms.add(crate::plan::with_children(&kind, &new), ty)
}

/// Returns `v`, of type `ty`, with every placeholder replaced by the value `draw` gives it.
///
/// A placeholder is a record where `ty` holds no record: a base value, or the key of a relation
/// whose key is one field.
pub fn fill(schema: &Schema, ty: &Ty, v: Val, draw: &mut dyn FnMut(&Val) -> Val) -> Val {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Base(_), v @ Val::Record(_)) => draw(&v),
        (Ty::Key { rel, .. }, v) => {
            let r = schema.relation(*rel);
            let row = &ctors(schema, &r.elem).expect("a relation's element is a sum")[0];
            let field_ty = |n: &str| {
                row.iter()
                    .find(|(f, _)| f == n)
                    .expect("a key field")
                    .1
                    .clone()
            };
            match r.key.as_slice() {
                [k] => match field_ty(k) {
                    Ty::Key { rel: own, .. } if own == *rel => match v {
                        v @ Val::Record(_) => draw(&v),
                        v => v,
                    },
                    f => fill(schema, &f, v, draw),
                },
                ks => {
                    let mut names = ks.to_vec();
                    names.sort();
                    let Val::Record(fs) = v else {
                        panic!("a composite key is a record")
                    };
                    Val::record(
                        names
                            .iter()
                            .zip(fs.iter())
                            .map(|(n, x)| fill(schema, &field_ty(n), x.clone(), draw))
                            .collect(),
                    )
                }
            }
        }
        (Ty::Set(e) | Ty::NonEmpty(e), Val::Set(s)) => {
            Val::set(s.iter().map(|x| fill(schema, e, x.clone(), draw)).collect())
        }
        (Ty::Record(fts), Val::Record(fs)) => Val::record(
            fts.iter()
                .zip(fs.iter())
                .map(|((_, t), x)| fill(schema, t, x.clone(), draw))
                .collect(),
        ),
        (_, Val::Ctor(c, fs)) => match ctors(schema, &t) {
            Some(cs) => Val::ctor(
                c,
                cs[c as usize]
                    .iter()
                    .zip(fs.iter())
                    .map(|((_, t), x)| fill(schema, t, x.clone(), draw))
                    .collect(),
            ),
            None => Val::Ctor(c, fs),
        },
        (_, v) => v,
    }
}
