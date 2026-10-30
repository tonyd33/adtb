use std::collections::{BTreeMap, HashMap};

use adtb_lang::kernel::{Const, Kind, Terms, Tm, Var};
use adtb_lang::schema::{Generator, Schema};
use adtb_lang::types::{Base, RelId, Ty};

use crate::exec::Val;
use crate::exec::types::{ctors, repr};

/// The generator and relation of each `Fresh` occurrence, by its placeholder's number.
pub type Occurrences = BTreeMap<i64, (Generator, RelId)>;

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
    for (c, b) in cs.iter().zip(kind.bound()) {
        let n = scope.len();
        scope.extend(b);
        new.push(walk(terms, *c, scope, memo, out));
        scope.truncate(n);
    }
    terms.add(kind.with_children(&new), ty)
}

/// Returns `v`, of type `ty`, with every placeholder replaced by the value `draw` gives it.
///
/// A placeholder is a record where `ty` holds no record: a base value, or the key of a relation
/// whose key is one field.
pub fn fill<E>(
    schema: &Schema,
    ty: &Ty,
    v: Val,
    draw: &mut dyn FnMut(&Val) -> Result<Val, E>,
) -> Result<Val, E> {
    let t = repr(schema, ty);
    Ok(match (&t, v) {
        (Ty::Base(_), v @ Val::Record(_)) => draw(&v)?,
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
                        v @ Val::Record(_) => draw(&v)?,
                        v => v,
                    },
                    f => fill(schema, &f, v, draw)?,
                },
                ks => {
                    let mut names = ks.to_vec();
                    names.sort();
                    let Val::Record(fs) = v else {
                        panic!("a composite key is a record")
                    };
                    let tys: Vec<Ty> = names.iter().map(|n| field_ty(n)).collect();
                    Val::record(fill_all(schema, tys.iter().zip(fs.iter()), draw)?)
                }
            }
        }
        (Ty::Set(e) | Ty::NonEmpty(e), Val::Set(s)) => Val::set(
            fill_all(schema, s.iter().map(|x| (e.as_ref(), x)), draw)?
                .into_iter()
                .collect(),
        ),
        (Ty::Record(fts), Val::Record(fs)) => Val::record(fill_all(
            schema,
            fts.iter().map(|(_, t)| t).zip(fs.iter()),
            draw,
        )?),
        (_, Val::Ctor(c, fs)) => match ctors(schema, &t) {
            Some(cs) => Val::ctor(
                c,
                fill_all(
                    schema,
                    cs[c as usize].iter().map(|(_, t)| t).zip(fs.iter()),
                    draw,
                )?,
            ),
            None => Val::Ctor(c, fs),
        },
        (_, v) => v,
    })
}

fn fill_all<'v, E>(
    schema: &Schema,
    vs: impl Iterator<Item = (&'v Ty, &'v Val)>,
    draw: &mut dyn FnMut(&Val) -> Result<Val, E>,
) -> Result<Vec<Val>, E> {
    vs.map(|(t, x)| fill(schema, t, x.clone(), draw)).collect()
}
