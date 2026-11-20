use super::compile;
use crate::plan::fixture::Fixture;

#[test]
fn compiling_checks_over_transitions_only_adds_terms_to_the_arena() {
    let f = Fixture::new(
        "type Event = { id: Key events }
relation events : Event key id generate sequence
constraint append_only = from e in deleted(events) select e
constraint grown = from p in prior(events) where not exists n in events where n.id == p.id select p
",
    );
    let mut terms = f.normal.terms().clone();
    compile(&f.schema, &f.normal, &mut terms, &f.catalog)
        .unwrap_or_else(|e| panic!("the constraints are admitted: {e:?}"));
    let kept = f.normal.terms().len();
    assert!(
        terms.iter().take(kept).eq(f.normal.terms().iter()),
        "a node of the normal arena changed"
    );
}
