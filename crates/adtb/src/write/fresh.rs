use std::collections::{BTreeMap, HashMap};

use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::schema::Generator;
use adtb_lang::types::RelId;

use crate::exec::Val;

/// The generator and relation of each `Fresh` occurrence, by its placeholder's number.
pub type Occurrences = BTreeMap<u32, (Generator, RelId)>;

/// Returns `t` with each path to a `Fresh` given its own placeholder, numbered, over the
/// variables bound around it, so that its value is one per evaluation. Adds the
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
    if let Kind::Block(..) = terms.kind(t) {
        let mut statements = Vec::new();
        let mut at = t;
        while let Kind::Block(w, k) = *terms.kind(at) {
            statements.push((at, w));
            at = k;
        }
        let mut h = holds_fresh(terms, at, memo);
        for (block, w) in statements.into_iter().rev() {
            h = holds_fresh(terms, w, memo) || h;
            memo.insert(block, h);
        }
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
        let (generator, rel, ty) = (*g, *r, terms.ty(t).clone());
        let occurrence =
            u32::try_from(terms.nodes.len()).expect("an arena of fewer than 2^32 nodes");
        let scope = scope
            .clone()
            .into_iter()
            .map(|v| {
                let vty = terms.var_info(v).ty.clone();
                terms.add_unchecked(Kind::Var(v), vty)
            })
            .collect();
        let p = terms.add_unchecked(
            Kind::Placeholder {
                occurrence,
                generator,
                rel,
                scope,
            },
            ty,
        );
        out.insert(occurrence, (generator, rel));
        return p;
    }
    if let Kind::Block(..) = terms.kind(t) {
        let mut statements = Vec::new();
        let mut at = t;
        while let Kind::Block(w, k) = *terms.kind(at) {
            let ty = terms.ty(at).clone();
            statements.push((walk(terms, w, scope, memo, out), ty));
            at = k;
        }
        let rest = walk(terms, at, scope, memo, out);
        return statements.into_iter().rev().fold(rest, |k, (w, ty)| {
            terms.add_unchecked(Kind::Block(w, k), ty)
        });
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
    terms.add_unchecked(kind.with_children(&new), ty)
}

/// Returns `v` with every placeholder replaced by the value `draw` gives it.
pub fn fill<E>(v: Val, draw: &mut Draw<'_, E>) -> Result<Val, E> {
    Ok(match v {
        p @ Val::Placeholder { occurrence, .. } => draw(occurrence, &p)?,
        Val::Set(s) => Val::set(fill_all(s.iter(), draw)?.into_iter().collect()),
        Val::Record(fs) => Val::record(fill_all(fs.iter(), draw)?),
        Val::Ctor(c, fs) => Val::ctor(c, fill_all(fs.iter(), draw)?),
        Val::List(xs) => Val::List(fill_all(xs.iter(), draw)?.into()),
        v => v,
    })
}

/// A source of placeholder values, taking a placeholder's occurrence number and the placeholder.
pub type Draw<'d, E> = dyn FnMut(u32, &Val) -> Result<Val, E> + 'd;

fn fill_all<'v, E>(
    vs: impl Iterator<Item = &'v Val>,
    draw: &mut Draw<'_, E>,
) -> Result<Vec<Val>, E> {
    vs.map(|x| fill(x.clone(), draw)).collect()
}
