use super::{Part, compile};
use crate::plan::fixture::Fixture;

#[test]
fn each_check_is_labelled_by_the_constructor_relation_and_part_that_runs_it() {
    let f = Fixture::new(
        "type User = { id: Key users }
relation users : User key id generate sequence
type Login { id: Key logins, user: Key users } = Email { email: Text } | Phone { phone: Text }
relation logins : Login key id generate sequence index (user)
constraint reachable =
  from p in logins.Phone where not exists e in logins.Email where e.user == p.user select p
",
    );
    let mut terms = f.normal.terms.clone();
    let checks = compile(&f.schema, &f.normal, &mut terms, &f.catalog)
        .unwrap_or_else(|e| panic!("the constraint is admitted: {e:?}"));
    let [c] = checks.as_slice() else {
        panic!("one constraint compiles: {checks:?}");
    };
    assert_eq!(c.name, "app::reachable");
    let triggers: Vec<_> = c
        .deltas
        .iter()
        .map(|d| d.explain(&f.schema, &f.catalog).trigger)
        .collect();
    assert_eq!(
        triggers,
        [
            Some(("app::logins.Phone".to_owned(), Part::Inserted)),
            Some(("app::logins.Email".to_owned(), Part::Changed)),
        ]
    );
    for d in &c.deltas {
        let shown = d.explain(&f.schema, &f.catalog).plan.text();
        assert!(
            !shown.contains("Scan"),
            "a check reads through probes:\n{shown}"
        );
    }
}

#[test]
fn a_constraint_over_both_constructors_has_a_check_per_constructor() {
    let f = Fixture::new(
        "type User = { id: Key users }
relation users : User key id generate sequence
type Login { id: Key logins, user: Key users } = Email { email: Text } | Phone { phone: Text }
relation logins : Login key id generate sequence index (user)
constraint single =
  from e in logins.Email, p in logins.Phone where e.user == p.user select e
",
    );
    let mut terms = f.normal.terms.clone();
    let checks = compile(&f.schema, &f.normal, &mut terms, &f.catalog)
        .unwrap_or_else(|e| panic!("the constraint is admitted: {e:?}"));
    let triggers: Vec<_> = checks[0]
        .deltas
        .iter()
        .map(|d| d.explain(&f.schema, &f.catalog).trigger)
        .collect();
    assert_eq!(
        triggers,
        [
            Some(("app::logins.Email".to_owned(), Part::Inserted)),
            Some(("app::logins.Phone".to_owned(), Part::Inserted)),
        ]
    );
}

#[test]
fn a_condition_on_a_branching_argument_is_admitted_through_probes() {
    for same in ["same", "same2"] {
        let f = Fixture::new(&format!(
            "type Membership = {{ id: Key memberships, user: Text, org: Text }}
relation memberships : Membership key id generate sequence index (org)
type Assignment = {{ id: Key assignments, mine: Bool, user: Text, org: Text, alt: Text }}
relation assignments : Assignment key id generate sequence index (user)
fn same (a: Text, b: Text) -> Bool = a == b
fn same2 (a: Text, b: Text) -> Bool = a == b and b == a
constraint assigned_member =
  from a in assignments
  where not exists m in memberships
    where m.user == a.user and {same}(m.org, if a.mine then a.org else a.alt)
  select a
"
        ));
        let mut terms = f.normal.terms.clone();
        let checks = compile(&f.schema, &f.normal, &mut terms, &f.catalog)
            .unwrap_or_else(|e| panic!("`{same}`: the constraint is admitted: {e:?}"));
        let shown: Vec<String> = checks[0]
            .deltas
            .iter()
            .map(|d| d.explain(&f.schema, &f.catalog).plan.text())
            .collect();
        assert!(!shown.is_empty(), "`{same}`: the constraint has checks");
        for s in &shown {
            assert!(
                s.contains("Probe memberships (org)") && !s.contains("Scan"),
                "`{same}`: a check reads through probes:\n{s}"
            );
        }
        assert!(
            shown.iter().any(|s| s.contains("Probe assignments (user)")),
            "`{same}`: a check on memberships probes assignments:\n{}",
            shown.join("\n")
        );
    }
}

#[test]
fn compiling_checks_over_transitions_only_adds_terms_to_the_arena() {
    let f = Fixture::new(
        "type Event = { id: Key events }
relation events : Event key id generate sequence
constraint append_only = from e in deleted(events) select e
constraint grown = from p in prior(events) where not exists n in events where n.id == p.id select p
",
    );
    let mut terms = f.normal.terms.clone();
    compile(&f.schema, &f.normal, &mut terms, &f.catalog)
        .unwrap_or_else(|e| panic!("the constraints are admitted: {e:?}"));
    let kept = f.normal.terms.len();
    assert!(
        terms.iter().take(kept).eq(f.normal.terms.iter()),
        "a node of the normal arena changed"
    );
}
