//! A `redefine fn` or `redefine computed` over the simulator: each index, unique, constraint,
//! check and domain that reaches the redefined body is rebuilt or validated at the migration.

use std::collections::BTreeSet;

use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::run;
use adtb_testkit::{profile, refused_as};

use crate::catalog::change;
use crate::catalog::ids::rel_id;
use crate::plan::HeapId;
use crate::test_world::api::{printed, query};
use crate::test_world::load::{self as open, options};
use crate::{CallOptions, Database, Error, Log, OpenError, OpenOptions, RejectReason};

/// A schema object that holds state over the rows of `items` and calls an expression of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dependent {
    Unique,
    Index,
    Check,
    Constraint,
    /// The domain of `Small`, the type of the field `s`, which holds the value of `n`.
    Domain,
}

impl Dependent {
    const ALL: [Dependent; 5] = [
        Dependent::Unique,
        Dependent::Index,
        Dependent::Check,
        Dependent::Constraint,
        Dependent::Domain,
    ];

    /// Returns the operator adding the dependent over `via`; empty for a domain, which the
    /// declaration of `Small` carries.
    fn operator(self, via: Via) -> String {
        match self {
            Dependent::Unique => format!("add unique ({}) in items", via.expression("")),
            Dependent::Index => format!("add index ({}) in items", via.expression("")),
            Dependent::Check => {
                format!("add check bounded on items ({} < 100)", via.expression(""))
            }
            Dependent::Constraint => format!(
                "add constraint bounded =\n  from i in items where {} >= 100 select i",
                via.expression("i.")
            ),
            Dependent::Domain => String::new(),
        }
    }

    /// Returns the domain of `Small`.
    fn domain(self, via: Via) -> String {
        match self {
            Dependent::Domain => format!(" where {} < 100", via.over("self", "")),
            _ => String::new(),
        }
    }
}

/// How the dependent's expression reaches the redefined body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Via {
    /// A call of `f`.
    Direct,
    /// A call of `g`, which calls `f`.
    Function,
    /// A function value calling `f`, passed to `apply`.
    Lambda,
    /// A call of `summed`, which reaches `f` only through the step of the aggregate `total`.
    Aggregate,
    /// The computed field `c`, which calls `f`.
    Computed,
    /// The computed field `d`, itself redefined.
    Recomputed,
}

impl Via {
    const ALL: [Via; 6] = [
        Via::Direct,
        Via::Function,
        Via::Lambda,
        Via::Aggregate,
        Via::Computed,
        Via::Recomputed,
    ];

    /// Returns the expression over a row whose fields are read as `row` followed by the name.
    fn expression(self, row: &str) -> String {
        self.over(&format!("{row}n"), row)
    }

    /// Returns the expression over the value `x`, the fields of a row read as `row` followed by
    /// the name.
    fn over(self, x: &str, row: &str) -> String {
        match self {
            Via::Direct => format!("f({x})"),
            Via::Function => format!("g({x})"),
            Via::Lambda => format!("apply(fn y -> f(y), {x})"),
            Via::Aggregate => format!("summed({x})"),
            Via::Computed => format!("{row}c + 0"),
            Via::Recomputed => format!("{row}d + 0"),
        }
    }

    /// Returns the expression over a row as the catalog prints an index element.
    fn canonical(self) -> &'static str {
        match self {
            Via::Direct => "app::f ( n )",
            Via::Function => "app::g ( n )",
            Via::Lambda => "app::apply ( fn y -> app::f ( y ) , n )",
            Via::Aggregate => "app::summed ( n )",
            Via::Computed => "c + 0",
            Via::Recomputed => "d + 0",
        }
    }

    /// Returns whether dependent `d` can read the expression: a domain reads a value, not a
    /// row, and only a constraint, a query, runs an aggregate.
    fn reaches(self, d: Dependent) -> bool {
        match self {
            Via::Computed | Via::Recomputed => d != Dependent::Domain,
            Via::Aggregate => d == Dependent::Constraint,
            Via::Direct | Via::Function | Via::Lambda => true,
        }
    }
}

/// A new body of `f`, or of `d` for [`Via::Recomputed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Body {
    /// The body it already has.
    Same,
    /// A body under which the rows of `items` keep distinct values under 100.
    Holds,
    /// A body under which every row of `items` has the value 100.
    Breaks,
}

impl Body {
    /// Returns the body over `x`.
    fn text(self, x: &str) -> String {
        match self {
            Body::Same => format!("{x} * 2"),
            Body::Holds => format!("{x} + 10"),
            Body::Breaks => format!("{x} * 0 + 100"),
        }
    }

    /// Returns the rows of the query of [`case`] under this body.
    fn rows(self) -> Vec<String> {
        let values = match self {
            Body::Same => [20, 30],
            Body::Holds => [20, 25],
            Body::Breaks => [100, 100],
        };
        [10, 15]
            .iter()
            .zip(values)
            .map(|(n, v)| format!("{{ n: {n}, v: {v} }}"))
            .collect()
    }
}

/// Returns the first migration: the schema with dependent `d` over `via`.
fn declared(d: Dependent, via: Via) -> String {
    format!(
        "module app
add module app
define fn f (x: I64) -> I64 = x * 2
define fn g (x: I64) -> I64 = f(x)
define fn apply (h: I64 -> I64, x: I64) -> I64 = h(x)
define aggregate total (x: I64) -> I64 =
  state {{ n: I64 }}
  init {{ n: 0 }}
  step (s, x) -> {{ n: s.n + f(x) }}
  merge (a, b) -> {{ n: a.n + b.n }}
  finish s -> s.n
define fn summed (x: I64) -> I64 = match total(v for v in [x]) | None -> 0 | Some s -> s.value
add type Small = new I64{}
add type Item = {{ id: Key items, n: I64, s: Small }}
add relation items : Item key id generate sequence
add computed items.c: I64 = f(self.n)
add computed items.d: I64 = {}
{}
",
        d.domain(via),
        Body::Same.text("self.n"),
        d.operator(via)
    )
}

const ROWS: &str = "transact
  insert items { n: 10, s: Small(10) }
  insert items { n: 15, s: Small(15) }";

const BOTH: &str = "Item { id: items.key(1), n: 10, s: Small(10) }, \
                    Item { id: items.key(2), n: 15, s: Small(15) }";

/// Returns the migration giving the body `via` reaches the new body `body`.
fn redefined(via: Via, body: Body) -> String {
    match via {
        Via::Recomputed => format!(
            "module app\nredefine computed items.d: I64 = {}\n",
            body.text("self.n")
        ),
        _ => format!(
            "module app\nredefine fn f (x: I64) -> I64 = {}\n",
            body.text("x")
        ),
    }
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

/// Returns the refusal of a migration with `detail`.
fn refusal(detail: &str) -> Error {
    Error::Rejected {
        reason: RejectReason::SchemaRefused {
            operator: String::new(),
            detail: detail.to_string(),
        },
    }
}

/// Assert that `got` is the refusal with `detail`.
fn refused(got: Error, detail: &str) {
    let want = refusal(detail);
    refused_as(Err::<(), _>(got), &want, &want.to_string());
}

/// The heaps of a database.
#[derive(Debug, PartialEq, Eq)]
struct Heaps {
    items: HeapId,
    /// The heap of every constructor of every relation.
    all: BTreeSet<HeapId>,
}

fn heaps(db: &Database<SimRuntime>) -> Heaps {
    let w = db.tested_db();
    Heaps {
        items: w
            .heap_named("app::items", "Item")
            .expect("items has a heap"),
        all: w.heap_ids().into_iter().collect(),
    }
}

/// What reopening with the second migration did.
#[derive(Debug)]
enum Outcome {
    Refused(Error),
    /// Applied, with the heaps the database then has and the rows of the query of [`case`], which,
    /// with the verification, runs only when the reopen leaves the query memory unbounded.
    Applied {
        heaps: Heaps,
        rows: Option<Vec<String>>,
    },
}

/// Returns the heaps of a database of `d` over `via` holding the rows of `ROWS`, and
/// what reopening it under `reopen` with the body `via` reaches redefined as `body` does.
/// Assert that an applied migration under unbounded query memory leaves index keys that match
/// the heaps.
fn case(seed: u64, d: Dependent, via: Via, body: Body, reopen: &OpenOptions) -> (Heaps, Outcome) {
    let first = declared(d, via);
    let second = redefined(via, body);
    let at = format!("{d:?} via {via:?} under {body:?}, seed {seed}");
    let mut sim = Sim::new(seed, Config::default());
    let mut before = None;
    run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &Log::new([first.as_str()]), &options(1));
        write(&db, ROWS);
        before = Some(heaps(&db));
    });
    let mut outcome = None;
    run(&mut sim, |rt| {
        let log = Log::new([first.as_str(), second.as_str()]);
        outcome = Some(match open::open_in(&rt, open::FILE, &log, reopen) {
            Err(OpenError::MigrationRefused { version, reason }) => {
                assert_eq!(2, version, "{at}");
                Outcome::Refused(reason)
            }
            Err(e) => panic!("{at}: refused otherwise: {e:?}"),
            Ok(db) => {
                let rows = reopen.query_memory_bytes.is_none().then(|| {
                    let findings = printed(db.verify(Some("app::items")).expect("verify runs"));
                    assert_eq!(Vec::<String>::new(), findings, "{at}: the index keys");
                    let text = format!(
                        "from i in items select {{ n: i.n, v: {} }}",
                        via.expression("i.")
                    );
                    query(&db, &text)
                });
                Outcome::Applied {
                    heaps: heaps(&db),
                    rows,
                }
            }
        });
    });
    match (before, outcome) {
        (Some(before), Some(outcome)) => (before, outcome),
        _ => panic!("{at}: a run ended early"),
    }
}

/// Returns the refusal a migration meets that gives both rows the value 100 under `d` over
/// `via`; `None` for an index, which no value refuses.
fn violated(d: Dependent, via: Via) -> Option<String> {
    match d {
        Dependent::Unique => Some(format!(
            "rows of items share the key 100 of `unique ({})`",
            via.canonical()
        )),
        Dependent::Index => None,
        Dependent::Check | Dependent::Constraint => {
            Some(format!("constraint app::bounded is violated by {BOTH}"))
        }
        Dependent::Domain => Some(format!(
            "constraint app::Small.domain is violated by {BOTH}"
        )),
    }
}

/// Returns the dependents and vias the search runs: every pair whose dependent can read the via
/// under the nextest profile `deep`, else enough pairs to run every dependent and every via.
fn pairs() -> BTreeSet<(Dependent, Via)> {
    if profile::every_case() {
        return Dependent::ALL
            .into_iter()
            .flat_map(|d| Via::ALL.map(|v| (d, v)))
            .filter(|&(d, v)| v.reaches(d))
            .collect();
    }
    BTreeSet::from([
        (Dependent::Unique, Via::Computed),
        (Dependent::Index, Via::Recomputed),
        (Dependent::Check, Via::Function),
        (Dependent::Constraint, Via::Aggregate),
        (Dependent::Domain, Via::Lambda),
        (Dependent::Unique, Via::Direct),
    ])
}

#[test]
fn a_changed_body_rebuilds_or_validates_each_dependent_reaching_it() {
    let pairs = pairs();
    let ds: BTreeSet<Dependent> = pairs.iter().map(|&(d, _)| d).collect();
    let vs: BTreeSet<Via> = pairs.iter().map(|&(_, v)| v).collect();
    assert_eq!(
        BTreeSet::from(Dependent::ALL),
        ds,
        "setup: every dependent runs"
    );
    assert_eq!(BTreeSet::from(Via::ALL), vs, "setup: every via runs");
    assert!(
        pairs.iter().all(|&(d, v)| v.reaches(d)),
        "setup: every pair can run"
    );
    let mut reached = BTreeSet::new();
    for (seed, &(d, via)) in pairs.iter().enumerate() {
        let seed = 2 * seed as u64;
        let at = format!("{d:?} via {via:?}");
        match case(seed, d, via, Body::Holds, &options(1)).1 {
            Outcome::Applied { rows, .. } => assert_eq!(Some(Body::Holds.rows()), rows, "{at}"),
            Outcome::Refused(e) => panic!("{at} under Holds is refused: {e}"),
        }
        match (
            violated(d, via),
            case(seed + 1, d, via, Body::Breaks, &options(1)),
        ) {
            (Some(detail), (_, Outcome::Refused(e))) => {
                refused(e, &detail);
                reached.insert((d, via));
            }
            (None, (before, Outcome::Applied { heaps, rows })) => {
                assert_eq!(Some(Body::Breaks.rows()), rows, "{at}");
                if heaps.items != before.items {
                    reached.insert((d, via));
                }
            }
            (want, (_, got)) => panic!("{at} under Breaks: expected {want:?}, got {got:?}"),
        }
    }
    assert_eq!(pairs, reached);
}

/// Returns `source` migrated by `text`.
fn migrated(source: &Source, text: &str) -> Source {
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    source.migrate(&file, text).expect("the migration applies")
}

#[test]
fn a_redefinition_rewrites_the_relation_of_a_dependent_index_only_when_the_body_changes() {
    for via in Via::ALL.into_iter().filter(|v| v.reaches(Dependent::Index)) {
        let before = migrated(&Source::default(), &declared(Dependent::Index, via));
        let (old, old_program) = before.check().expect("the source checks");
        let items = rel_id(
            &old,
            *old.module(old.module_named("app").expect("app"))
                .values
                .get("items")
                .map(|v| match v {
                    adtb_lang::schema::ValueDef::Relation(r) => r,
                    _ => panic!("items is a relation"),
                })
                .expect("items"),
        );
        for body in [Body::Same, Body::Holds, Body::Breaks] {
            let after = migrated(&before, &redefined(via, body));
            let (new, new_program) = after.check().expect("the source checks");
            let want = match body {
                Body::Same => BTreeSet::new(),
                Body::Holds | Body::Breaks => BTreeSet::from([items]),
            };
            assert_eq!(
                want,
                change::rewritten(
                    &old,
                    &new,
                    &adtb_lang::depends::changed(&old, &old_program, &new, &new_program),
                ),
                "{via:?} under {body:?}"
            );
        }
    }
}
