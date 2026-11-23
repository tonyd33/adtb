use super::*;

/// A computed field ranges over every constructor of its relation: a clause scoped to some of
/// them, before or after it, and a read of it through a row narrowed to some, leave its match
/// over all of them checking.
#[test]
fn a_computed_match_checks_beside_every_clause_scope_and_through_every_narrowed_row() {
    let clauses = [
        "",
        "  unique (x) in A\n",
        "  unique (x) in (A | B)\n",
        "  index (x) in B\n",
        "  index (x) in (A | B)\n",
    ];
    let computed = "  computed label: Text = match self | A a -> a.x | B b -> b.x | C c -> c.z\n";
    let scopes = ["", ".A", ".B", ".C", ".(A | B)", ".(B | C)", ".(A | C)"];
    let mut ran = 0;
    for clause in clauses {
        for before in [true, false] {
            let (first, second) = if before {
                (clause, computed)
            } else {
                (computed, clause)
            };
            let extra = format!(
                "type T {{ id: Key ts }} = A {{ x: Text }} | B {{ x: Text, y: Text }} | C {{ z: Text }}\n\
                 relation ts : T\n  key id generate sequence\n{first}{second}"
            );
            accepts(&extra);
            for scope in scopes {
                let text = format!("query q () -> Set Text = from t in ts{scope} select t.label");
                if let Err(e) = query(&extra, &text) {
                    panic!("{extra}\n{text}\nrefused: {e}");
                }
                ran += 1;
            }
        }
    }
    assert_eq!(clauses.len() * 2 * scopes.len(), ran);
}
