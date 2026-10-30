use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn int() -> Ty {
    Ty::Base(Base::I64)
}

fn free(terms: &Terms, t: Tm) -> BTreeSet<Var> {
    (*Frees::default().of(terms, t)).clone()
}

#[test]
fn a_let_variable_read_in_its_own_value_is_free() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let read = terms.add(Kind::Var(x), int());
    let t = terms.add(Kind::Let(x, read, read), int());
    assert_eq!(free(&terms, t), BTreeSet::from([x]));
}

#[test]
fn a_for_variable_read_in_its_own_source_is_free() {
    let mut terms = Terms::default();
    let set = Ty::Set(Box::new(int()));
    let x = terms.var("x", set.clone());
    let read = terms.add(Kind::Var(x), set.clone());
    let t = terms.add(Kind::For(x, read, read), set);
    assert_eq!(free(&terms, t), BTreeSet::from([x]));
}

#[test]
fn a_variable_bound_around_every_child_is_not_free() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let read = terms.add(Kind::Var(x), int());
    let lam = terms.add(Kind::Lam(x, read), Ty::Fn(Box::new(int()), Box::new(int())));
    let fix = terms.add(Kind::Fix(vec![(x, read)], read), int());
    assert_eq!(free(&terms, lam), BTreeSet::new());
    assert_eq!(free(&terms, fix), BTreeSet::new());
}

#[test]
fn renaming_copies_a_term_whose_binder_is_free_in_its_own_value() {
    let mut terms = Terms::default();
    let x = terms.var("x", int());
    let y = terms.var("y", int());
    let read = terms.add(Kind::Var(x), int());
    let test = terms.add(Kind::Const(Const::Int(1)), int());
    let inner = terms.add(Kind::Let(x, read, read), int());
    let t = terms.add(Kind::If(test, inner, inner), int());
    let c = terms.renamed(t, &BTreeMap::from([(x, y)]), &mut Frees::default());
    assert_ne!(c, t);
    assert_eq!(free(&terms, c), BTreeSet::from([y]));
}
