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

/// An index a shape's incremental check needs, and the refusal of the shape without it.
struct Need {
    relation: &'static str,
    index: &'static str,
    refusals: &'static [&'static str],
}

/// A constraint of the within-bound grammar, the indexes its check needs, and its refusal when
/// it is outside the bound whatever the indexes.
struct Shape {
    name: &'static str,
    constraint: &'static str,
    needs: &'static [Need],
    outside: &'static [&'static str],
}

/// Returns the declarations every shape reads, each relation with the indexes `with` gives it.
fn declarations(with: &[&Need]) -> String {
    let indexes = |r: &str| -> String {
        with.iter()
            .filter(|n| n.relation == r)
            .map(|n| format!("\n  {}", n.index))
            .collect()
    };
    format!(
        "type User = {{ id: Key users, name: Text, org: Text }}
relation users : User key id generate sequence{}
type Ident {{ id: Key idents, owner: Text }} = Google | Mail {{ address: Text }}
relation idents : Ident key id generate sequence{}
type Line = {{ id: Key lines, entry: Text, amount: I64 }}
relation lines : Line key id generate sequence{}
type Booking = {{ id: Key bookings, resource: Text, during: Span Instant }}
relation bookings : Booking key id generate sequence{}
type Grant {{ id: Key grants, org: Text }} = ToUser {{ user: Text }} | ToIdent {{ ident: Text }}
relation grants : Grant key id generate sequence{}
",
        indexes("users"),
        indexes("idents"),
        indexes("lines"),
        indexes("bookings"),
        indexes("grants"),
    )
}

/// Returns each constraint of `src` without an index-bounded check, as its refusals print.
fn admission(src: &str) -> Vec<String> {
    let f = Fixture::new(src);
    let mut terms = f.normal.terms().clone();
    match compile(&f.schema, &f.normal, &mut terms, &f.catalog) {
        Ok(_) => Vec::new(),
        Err(super::Unadmitted::Refused(rs)) => rs
            .into_iter()
            .map(|r| format!("{}: {}", r.constraint, r.detail))
            .collect(),
        Err(super::Unadmitted::Internal(b)) => panic!("{src}: {b:?}"),
    }
}

const SHAPES: &[Shape] = &[
    Shape {
        name: "not exists correlated by a field",
        constraint: "from u in users where not exists i in idents where i.owner == u.name select u",
        needs: &[
            Need {
                relation: "idents",
                index: "index (owner)",
                refusals: &[
                    "changes to app::users: index (owner) in app::idents is missing",
                    "changes to app::idents: index (owner) in app::idents is missing",
                ],
            },
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &["changes to app::idents: index (name) in app::users is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "exists over a constructor set",
        constraint: "from u in users where exists i in idents.Mail where i.owner == u.name select u",
        needs: &[
            Need {
                relation: "idents",
                index: "index (owner) in Mail",
                refusals: &[
                    "changes to app::users: index (owner) in app::idents.Mail is missing",
                    "changes to app::idents.Mail: index (owner) in app::idents.Mail is missing",
                ],
            },
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &["changes to app::idents.Mail: index (name) in app::users is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "exists with a filter beside its correlation",
        constraint: "from u in users where exists i in idents.Mail where i.owner == u.name and i.address == \"\" select u",
        needs: &[
            Need {
                relation: "idents",
                index: "index (owner) in Mail",
                refusals: &[
                    "changes to app::users: index (owner, address) in app::idents.Mail is missing",
                    "changes to app::idents.Mail: index (owner, address) in app::idents.Mail is missing",
                ],
            },
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &["changes to app::idents.Mail: index (name) in app::users is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "sum by a group key",
        constraint: "from u in users where sum(l.amount for l in lines where l.entry == u.name) > 10 select u",
        needs: &[
            Need {
                relation: "lines",
                index: "index (entry)",
                refusals: &[
                    "changes to app::users: index (entry) in app::lines is missing",
                    "changes to app::lines: index (entry) in app::lines is missing",
                ],
            },
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &["changes to app::lines: index (name) in app::users is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "max within a group",
        constraint: "from u in users where match max(l.amount for l in lines where l.entry == u.name) | None -> false | Some m -> m.value > 10 select u",
        needs: &[
            Need {
                relation: "lines",
                index: "index (entry, amount)",
                refusals: &[
                    "changes to app::users: `max` of amount needs index (entry, amount) in app::lines",
                    "changes to app::users: index (entry) in app::lines is missing",
                    "changes to app::lines: `max` of amount needs index (entry, amount) in app::lines",
                    "changes to app::lines: index (entry) in app::lines is missing",
                ],
            },
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &["changes to app::lines: index (name) in app::users is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "exclusion over a range",
        constraint: "from a in bookings, b in bookings where a.resource == b.resource and overlaps(a.during, b.during) and key a != key b select { a: key a, b: key b }",
        needs: &[Need {
            relation: "bookings",
            index: "interval index (resource, during)",
            refusals: &[
                "changes to app::bookings: an exclusion under `overlaps` on during needs interval index (resource, during) in app::bookings",
                "changes to app::bookings: index (resource) in app::bookings is missing",
            ],
        }],
        outside: &[],
    },
    Shape {
        name: "exists in a nested match arm",
        constraint: "from g in grants where (match g | ToUser t -> exists u in users where u.name == t.user | ToIdent t -> false) select g",
        needs: &[
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &[
                    "changes to app::grants.ToUser: index (name) in app::users is missing",
                    "changes to app::users: index (name) in app::users is missing",
                ],
            },
            Need {
                relation: "grants",
                index: "index (user) in ToUser",
                refusals: &["changes to app::users: index (user) in app::grants.ToUser is missing"],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "a correlation through a match",
        constraint: "from g in grants where exists u in users where u.org != g.org and u.name == (match g | ToUser t -> t.user | ToIdent t -> t.ident) select g",
        needs: &[
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &[
                    "changes to app::grants.ToIdent: index (name) in app::users is missing",
                    "changes to app::grants.ToUser: index (name) in app::users is missing",
                    "changes to app::users: index (name) in app::users is missing",
                ],
            },
            Need {
                relation: "grants",
                index: "index (user) in ToUser",
                refusals: &["changes to app::users: index (user) in app::grants.ToUser is missing"],
            },
            Need {
                relation: "grants",
                index: "index (ident) in ToIdent",
                refusals: &[
                    "changes to app::users: index (ident) in app::grants.ToIdent is missing",
                ],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "reads in top-level match arms",
        constraint: "from g in grants, x in (match g | ToUser t -> (from u in users where u.name == t.user and u.org != t.org select u.name) | ToIdent t -> (from i in idents where i.owner == t.ident select i.owner)) select x",
        needs: &[
            Need {
                relation: "users",
                index: "index (name)",
                refusals: &[
                    "changes to app::grants.ToUser: index (name) in app::users is missing",
                    "changes to app::users: index (name) in app::users is missing",
                ],
            },
            Need {
                relation: "idents",
                index: "index (owner)",
                refusals: &[
                    "changes to app::grants.ToIdent: index (owner) in app::idents is missing",
                    "changes to app::idents: index (owner) in app::idents is missing",
                ],
            },
            Need {
                relation: "grants",
                index: "index (user) in ToUser",
                refusals: &["changes to app::users: index (user) in app::grants.ToUser is missing"],
            },
            Need {
                relation: "grants",
                index: "index (ident) in ToIdent",
                refusals: &[
                    "changes to app::idents: index (ident) in app::grants.ToIdent is missing",
                ],
            },
        ],
        outside: &[],
    },
    Shape {
        name: "a fixpoint",
        constraint: "fix reach = (from u in users select { a: u.name, b: u.org }) union (from r in reach, u in users where u.name == r.b select { a: r.a, b: u.org }) in from r in reach where r.a == r.b select r",
        needs: &[Need {
            relation: "users",
            index: "index (name)",
            refusals: &[],
        }],
        outside: &["changes to app::users: `fix` is outside the bound"],
    },
];

#[test]
fn a_shape_within_the_bound_is_admitted_and_one_without_an_index_it_needs_is_refused() {
    let mut reached = (0, 0);
    for shape in SHAPES {
        for missing in std::iter::once(None).chain((0..shape.needs.len()).map(Some)) {
            let with: Vec<&Need> = shape
                .needs
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != missing)
                .map(|(_, n)| n)
                .collect();
            let src = format!(
                "{}constraint shape = {}\n",
                declarations(&with),
                shape.constraint
            );
            let lines: &[&str] = match missing {
                _ if !shape.outside.is_empty() => shape.outside,
                None => &[],
                Some(i) => shape.needs[i].refusals,
            };
            let expected: Vec<String> = lines.iter().map(|l| format!("app::shape: {l}")).collect();
            let mut got = admission(&src);
            let mut want = expected.clone();
            got.sort();
            want.sort();
            assert_eq!(
                want,
                got,
                "{} without {:?}",
                shape.name,
                missing.map(|i| shape.needs[i].index)
            );
            if want.is_empty() {
                reached.0 += 1;
            } else {
                reached.1 += 1;
            }
        }
    }
    assert!(
        reached.0 > 0 && reached.1 > 0,
        "admitted {} and refused {}",
        reached.0,
        reached.1
    );
}
