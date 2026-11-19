use adtb_lang::kernel::{Const, Kind};

use super::TermKind;

#[test]
fn a_term_kind_holding_a_nan_constant_equals_itself() {
    let nan = TermKind(Kind::Const(Const::Float(f64::NAN)));
    assert_eq!(nan, nan.clone());
    assert_ne!(nan, TermKind(Kind::Const(Const::Float(1.0))));
}
