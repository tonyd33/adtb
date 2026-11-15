use adtb_lang::schema::build;

use crate::Fault;

#[test]
fn each_fault_is_the_prelude_constructor_of_its_name() {
    let all = [
        Fault::DivisionByZero,
        Fault::ArithmeticOverflow,
        Fault::OutOfRange,
        Fault::NotFinite,
        Fault::NoSuchDate,
        Fault::UnknownZone,
        Fault::InvalidText,
        Fault::InvalidSpan,
        Fault::NegativeCount,
        Fault::NoSuchRow,
        Fault::FunctionFailed,
    ];
    let schema = build(&[]).expect("the engine's declarations build");
    let def = schema.ty(schema.prelude().fault());
    assert_eq!(all.len(), def.ctors().len());
    for f in all {
        assert_eq!(f.name(), def.ctors()[f.ctor() as usize].name, "{f:?}");
    }
}
