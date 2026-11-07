use adtb::plan::fixture as planfix;

use adtb_simkit::support;

#[path = "constraints/names.rs"]
mod names;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb::catalog::{Catalog as Stored, Error as CatalogError};
use adtb::exec::{self, Budget, Val};
use adtb::mvcc::Limits;
use adtb::plan::Catalog;
use adtb::storage::Pager;
use adtb::write::{self, Bindings, Database, Rejection as WRejection, Setup};
use adtb_lang::check::check_adhoc;
use adtb_lang::kernel::{Terms, Tm};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use adtb_testkit::fixture::Fixture;
use support::{Db, World};

/// The running schema at version 12, the extension of the constraints part, and its migration
/// 0013 adding an index on the owner of every identity.
const SPEC: &str = r"
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
  index (user)
set OAuth of identities = Google | Apple
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
    | Apple a     -> scalar_len(a.sub) == 0
  select i

type Entry = { id: Key entries, posted: Instant }
relation entries : Entry key id generate sequence
type Line = { id: Key lines, entry: Key entries on delete cascade, amount: I64 }
relation lines : Line
  key id generate sequence
  index (entry)

type Booking { id: Key bookings, resource: Text, during: Span Instant }
  = Room { seats: I64 }
  | Desk
relation bookings : Booking
  key id generate sequence
  interval index (resource, during) in (Room | Desk)

type Account { id: Key accounts, owner: Key users, opened: Instant }
  = Open
  | Closed { closed: Instant }
relation accounts : Account key id generate sequence

type Event = { id: Key events, at: Instant, body: Text }
relation events : Event key id generate sequence

type Place { id: Key places }
  = US   { zip: Text, state: Text }
  | Intl { country: Text, postcode: Text }
relation places : Place
  key id generate sequence
  index (zip) in US

type Membership = { id: Key memberships, user: Key users, org: Text }
relation memberships : Membership
  key id generate sequence
  unique (user, org)
type Assignment = { id: Key assignments, user: Key users, org: Text, task: Text }
relation assignments : Assignment key id generate sequence
";

/// Returns each constraint of schema `src` that the engine refuses, with why.
fn refusals(src: &str) -> Vec<(String, String)> {
    let f = Fixture::new(src);
    let normal = norm::program(&f.schema, &f.program).expect("the schema normalises");
    let catalog = planfix::catalog(&f.schema);
    write::refusals(&f.schema, &normal, &catalog)
        .unwrap_or_else(|e| panic!("admission breaks a guarantee: {e:?}"))
}

/// Returns the one refusal of schema `src`.
#[must_use]
fn refused(src: &str) -> (String, String) {
    let why = refusals(src);
    match why.as_slice() {
        [one] => one.clone(),
        _ => panic!("not one refusal: {why:?}"),
    }
}

/// A sum whose constructors each hold their own reference field `parent`.
const PLACES: &str = r"
type Spot { id: Key spots, zone: Text }
  = Region { parent: Option (Key spots.(Region | Town)) }
  | Town   { parent: Option (Key spots.(Region | Town)) }
  | Mark   { label: Text }
relation spots : Spot key id generate sequence
";

#[test]
fn a_join_on_a_field_one_constructor_has_no_index_on_is_refused_naming_the_set() {
    let src = PLACES
        .replace("Option (Key spots.(Region | Town))", "Option Text")
        .replacen(
            "relation spots : Spot key id generate sequence",
            "relation spots : Spot key id generate sequence index (zone) index (parent) in Region",
            1,
        );
    let decl = "constraint parent_in_zone =
  from l in spots.(Region | Town), p in spots.(Region | Town)
  where l.parent == Some { value: p.zone } and key p != key l
  select l";
    let (_, detail) = refused(&format!("{src}\n{decl}\n"));
    assert_eq!(
        "changes to app::spots.(Region | Town): index (parent) in app::spots.(Region | Town) is missing",
        detail
    );
}

#[test]
fn a_constraint_joining_on_a_field_no_index_leads_with_is_refused_naming_it() {
    let src = SPEC.replace(
        "{ id: Key identities, user: Key users }",
        "{ id: Key identities, user: Text }",
    );
    let decl = "constraint has_identity =
  from u in users
  where not exists i in identities where i.user == u.name
  select u";
    let why = refusals(&format!("{src}\n{decl}\n"));
    let name = |d: &str| ("app::has_identity".to_string(), d.to_string());
    assert_eq!(
        vec![name(
            "changes to app::identities: index (name) in app::users is missing"
        ),],
        why
    );
}

#[test]
fn an_inclusion_without_an_index_on_its_source_is_refused_naming_the_index() {
    let decl = "constraint assigned_member =
  from a in assignments
  where not exists m in memberships where m.user == a.user and m.org == a.org
  select a";
    let spec = SPEC.replace("user: Key users, org", "user: Text, org");
    let (c, detail) = refused(&format!("{spec}\n{decl}\n"));
    assert_eq!("app::assigned_member", c);
    assert_eq!(
        "changes to app::memberships: index (user, org) in app::assignments is missing",
        detail
    );
    let indexed = spec.replacen(
        "relation assignments : Assignment key id generate sequence",
        "relation assignments : Assignment key id generate sequence index (user, org)",
        1,
    );
    assert!(refusals(&format!("{indexed}\n{decl}\n")).is_empty());
}

#[test]
fn only_the_relations_whose_changes_are_unbounded_are_named() {
    let src = SPEC
        .replacen("  index (user)\n", "", 1)
        .replace(
            "{ id: Key identities, user: Key users }",
            "{ id: Key identities, user: Text }",
        )
        .replace("user: Key users, org", "user: Text, org")
        .replacen(
            "relation users : User key id generate sequence",
            "relation users : User key id generate sequence index (name)",
            1,
        );
    let decl = "constraint both =
  from u in users
  where not exists i in identities where i.user == u.name
    and not exists m in memberships where m.user == u.name and m.org == u.name
  select u";
    let why = refusals(&format!("{src}\n{decl}\n"));
    let name = |d: &str| ("app::both".to_string(), d.to_string());
    assert_eq!(
        vec![
            name("changes to app::users: index (user) in app::identities is missing"),
            name("changes to app::identities: index (user) in app::identities is missing"),
            name("changes to app::memberships: index (user) in app::identities is missing"),
        ],
        why
    );
}

#[test]
fn a_read_no_field_connects_to_the_change_is_refused() {
    let decl = r#"constraint quiet =
  from u in users
  where exists e in events where e.body == "stop"
  select u"#;
    let src = SPEC.replacen(
        "relation events : Event key id generate sequence",
        "relation events : Event key id generate sequence index (body)",
        1,
    );
    let (_, detail) = refused(&format!("{src}\n{decl}\n"));
    assert_eq!(
        "changes to app::events: no field connects them to app::users, so every row of it is read",
        detail
    );
}

#[test]
fn recursion_is_outside_the_bound() {
    let decl = "type Follow = { src: Key users, dst: Key users }
relation follows : Follow key (src, dst)
constraint no_cycle =
  fix reach = (from f in follows select { a: f.src, b: f.dst })
            union (from r in reach, f in follows where f.src == r.b select { a: r.a, b: f.dst })
  in from r in reach where r.a == r.b select r";
    let why = refusals(&format!("{SPEC}\n{decl}\n"));
    assert!(
        !why.is_empty()
            && why
                .iter()
                .all(|(c, d)| c == "app::no_cycle" && d.ends_with("`fix` is outside the bound")),
        "{why:?}"
    );
}

#[test]
fn min_and_max_need_an_index_on_the_field_within_the_group() {
    let decl = "constraint capped =
  from e in entries
  where match max(l.amount for l in lines where l.entry == key e)
    | None -> false
    | Some m -> m.value > 100
  select e";
    let why = refusals(&format!("{SPEC}\n{decl}\n"));
    assert!(
        !why.is_empty()
            && why
                .iter()
                .all(|(_, d)| d
                    .ends_with("`max` of amount needs index (entry, amount) in app::lines")),
        "{why:?}"
    );
    let indexed = SPEC.replacen("  index (entry)\n", "  index (entry, amount)\n", 1);
    assert!(refusals(&format!("{indexed}\n{decl}\n")).is_empty());
}

const NO_DOUBLE_BOOKING: &str = "constraint no_double_booking =
  from a in bookings.(Room | Desk), b in bookings.(Room | Desk)
  where a.resource == b.resource and overlaps(a.during, b.during) and key a != key b
  select { a: key a, b: key b }";

#[test]
fn an_exclusion_over_a_range_needs_an_interval_index_after_its_equality_fields() {
    let plain = SPEC.replacen(
        "interval index (resource, during) in (Room | Desk)",
        "index (resource) in (Room | Desk)",
        1,
    );
    let why = refusals(&format!("{plain}\n{NO_DOUBLE_BOOKING}\n"));
    assert!(
        !why.is_empty() && why.iter().all(|(_, d)| d.ends_with(
            "an exclusion under `overlaps` on during needs interval index (resource, during) in app::bookings"
        )),
        "{why:?}"
    );
}

const RUNNING: &str = r"module auth
add module auth
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
add relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
";

const HAS_IDENTITY: &str = "add constraint has_identity =
  from u in users
  where not exists i in identities where i.user == key u
  select u
";

/// Returns what the engine's catalog does with `first` as its first migration, then `second`.
fn migrate(first: &str, second: Option<&str>) -> Vec<Result<(), CatalogError>> {
    let mut sim = Sim::new(3, Config::default());
    let mut got = Vec::new();
    let outcome = sim.run(|rt| {
        let pager = Pager::create(rt.clone(), &adtb_simkit::harness::options(256)).expect("create");
        let (mut catalog, txns) = Stored::create(rt, &pager).expect("the catalog");
        got.push(catalog.migrate(&pager, &txns, first));
        if let Some(text) = second {
            got.push(catalog.migrate(&pager, &txns, text));
        }
    });
    assert_eq!(Outcome::Finished, outcome);
    got
}

#[test]
fn a_migration_declaring_a_constraint_outside_the_bound_is_refused() {
    let by_email = "add constraint by_email =
  from u in users
  where not exists i in identities.EmailPass where i.email == u.name
  select u
";
    let got = migrate(&format!("{RUNNING}{by_email}"), None);
    let Some(Err(CatalogError::Refused(why))) = got.first() else {
        panic!("the migration is not refused: {got:?}")
    };
    assert_eq!(
        vec![
            "constraint auth::by_email is not admitted: changes to auth::identities.EmailPass: \
             index (name) in auth::users is missing",
        ],
        *why
    );
    let indexed = RUNNING.replacen(
        "add relation users : User key id generate sequence\n",
        "add relation users : User key id generate sequence\n  index (name)\n",
        1,
    );
    let got = migrate(&format!("{indexed}{by_email}"), None);
    assert!(matches!(got.as_slice(), [Ok(())]), "{got:?}");
}

#[test]
#[ignore = "the engine stores no view, so it admits every `store`"]
fn storing_a_view_with_no_index_bounded_maintenance_is_refused_naming_the_index() {
    let view = "define fn by_name () -> Set ({ user: Key users, n: I64 }) =
  from u in users
  select { user: key u, n: count(i for i in identities.EmailPass where i.email == u.name) }
store by_name
";
    let got = migrate(&format!("{RUNNING}{view}"), None);
    let Some(Err(CatalogError::Refused(why))) = got.first() else {
        panic!("the migration is not refused: {got:?}")
    };
    assert!(
        why.iter()
            .any(|w| w.contains("by_name") && w.contains("index (name) in auth::users")),
        "{why:?}"
    );
    let indexed = RUNNING.replacen(
        "add relation users : User key id generate sequence\n",
        "add relation users : User key id generate sequence\n  index (name)\n",
        1,
    );
    let indexed = indexed.replacen(
        "  unique (email) in EmailPass\n",
        "  unique (email) in EmailPass\n  index (email) in EmailPass\n",
        1,
    );
    let got = migrate(&format!("{indexed}{view}"), None);
    assert!(matches!(got.as_slice(), [Ok(())]), "{got:?}");
}

#[test]
fn dropping_an_index_an_admitted_constraint_needs_is_refused_naming_the_constraint() {
    let by_email = "add constraint by_email =
  from u in users
  where not exists i in identities.EmailPass where i.email == u.name
  select u
";
    let indexed = RUNNING.replacen(
        "add relation users : User key id generate sequence\n",
        "add relation users : User key id generate sequence\n  index (name)\n",
        1,
    );
    let got = migrate(
        &format!("{indexed}{by_email}"),
        Some("module auth\ndrop index (name) in users\n"),
    );
    let Some(Err(CatalogError::Refused(why))) = got.get(1) else {
        panic!("the drop is not refused: {got:?}")
    };
    assert!(
        why.iter().any(|w| w.contains("auth::by_email")),
        "the refusal does not name the constraint: {why:?}"
    );
}

#[test]
fn dropping_the_declared_index_leading_a_reference_field_brings_back_the_implied_one() {
    let indexed = format!("{RUNNING}  index (user)\n{HAS_IDENTITY}");
    let got = migrate(
        &indexed,
        Some("module auth\ndrop index (user) in identities\n"),
    );
    assert!(matches!(got.as_slice(), [Ok(()), Ok(())]), "{got:?}");
}

/// An abort as the two sides are compared: what it is, and the rows it reports.
type Failed = (String, BTreeSet<Val>);

fn engine_error(schema: &Schema, e: &write::Error) -> Failed {
    match e {
        write::Error::Rejected(WRejection::Violated { constraint, report }) => {
            (format!("Violated {constraint}"), report.clone())
        }
        write::Error::Rejected(WRejection::KeyViolation { relation, .. }) => (
            format!("KeyViolation {}", schema.relation(*relation).name),
            BTreeSet::new(),
        ),
        other => (format!("{other:?}"), BTreeSet::new()),
    }
}

/// A statement's normal form.
struct Prepared {
    terms: Terms,
    item: Tm,
}

fn prepare(f: &Fixture, text: &str) -> Prepared {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("`{text}` does not parse"));
    let adhoc = check_adhoc(&f.schema, &f.program, &items, &BTreeMap::new())
        .unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
    let normal = norm::adhoc(&f.schema, &f.program, &adhoc)
        .unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
    Prepared {
        item: normal.items[0],
        terms: normal.terms,
    }
}

fn database(
    rt: &SimRuntime,
    f: &Fixture,
    normal: &Arc<NormalProgram>,
    catalog: &Arc<Catalog>,
) -> Database<SimRuntime> {
    let world = World {
        schema: &f.schema,
        normal,
        catalog,
        db: &f.db,
    };
    let (pager, txns, heaps, indexes) = Db::load_with(rt, &world, false).bindings();
    let sequences = support::counters(&pager, &f.schema, catalog, &indexes);
    Database::new(
        rt.clone(),
        Setup {
            schema: Arc::new(f.schema.clone()),
            normal: normal.clone(),
            catalog: catalog.clone(),
            pager,
            txns,
            bindings: Bindings { heaps, indexes },
            budget: Budget::new(1 << 30),
            grant: 64 << 20,
            pool: None,
            options: exec::Options::default(),
            limits: Limits::default(),
            sequences,
        },
    )
    .expect("a database over the fixture")
}

/// Runs `body` over a database of schema `src` in a simulation of `seed`.
fn with_engine<T>(
    seed: u64,
    src: &str,
    body: impl FnOnce(&Fixture, &Database<SimRuntime>) -> T,
) -> T {
    let f = Fixture::new(src);
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal = Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let mut got = None;
    let mut sim = Sim::new(seed, Config::default());
    let outcome = sim.run(|rt| {
        let db = database(&rt, &f, &normal, &catalog);
        got = Some(body(&f, &db));
    });
    assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    got.unwrap_or_else(|| panic!("seed {seed}: the run gave nothing"))
}

const PEOPLE: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence index (name)
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
relation identities : Identity key id generate sequence index (user) unique (gid) in Google
type Ban = { gid: Text }
relation bans : Ban key gid
";

const HAS_ONE: &str = "constraint has_identity =
  from u in users
  where not exists i in identities where i.user == key u
  select u
";

const ROSTER: &str = r"
type Membership = { id: Key memberships, user: Text, org: Text }
relation memberships : Membership key id generate sequence unique (user, org)
type Assignment = { id: Key assignments, user: Text, org: Text, task: Text }
relation assignments : Assignment key id generate sequence index (user, org)
constraint assigned_member =
  from a in assignments
  where not exists m in memberships where m.user == a.user and m.org == a.org
  select a
";

/// Runs `a` and `b` in two transactions in the order `order` gives, a sequence over `'a'` and
/// `'b'` for a statement and `'A'` and `'B'` for a commit. Returns how each ended.
fn interleave(
    f: &Fixture,
    db: &Database<SimRuntime>,
    a: &str,
    b: &str,
    order: &str,
) -> (Result<(), String>, Result<(), String>) {
    let pa = prepare(f, a);
    let pb = prepare(f, b);
    let mut ends: [Result<(), String>; 2] = [Ok(()), Ok(())];
    let mut open = [
        Some(db.begin().expect("a transaction")),
        Some(db.begin().expect("a transaction")),
    ];
    for step in order.chars() {
        let i = usize::from(step.eq_ignore_ascii_case(&'b'));
        if ends[i].is_err() {
            continue;
        }
        let tx = open[i].take().expect("an open transaction");
        if step.is_ascii_lowercase() {
            let p = if i == 0 { &pa } else { &pb };
            match db.statement(&tx, &p.terms, p.item, &[], 0) {
                Ok(_) => open[i] = Some(tx),
                Err(e) => {
                    ends[i] = Err(format!("{:?}", engine_error(&f.schema, &e)));
                    db.abort(tx).expect("an abort");
                }
            }
        } else if let Err(e) = db.commit(tx) {
            ends[i] = Err(format!("{:?}", engine_error(&f.schema, &e)));
        }
    }
    let [ra, rb] = ends;
    (ra, rb)
}

/// Asserts that after `setup`, two transactions running `a` and `b`, each of which keeps the
/// constraint whose query is `denial` alone, never both commit in any order of their
/// statements and commits, that a failure is a serialization failure, and that the constraint
/// holds afterwards.
fn serial(name: &str, src: &str, setup: &str, a: &str, b: &str, denial: &str) {
    for order in ["abAB", "abBA", "aAbB", "baAB", "baBA", "bBaA"] {
        for seed in 0..2 {
            let (ra, rb, left) = with_engine(seed, src, |f, db| {
                let p = prepare(f, setup);
                let tx = db.begin().expect("a transaction");
                db.statement(&tx, &p.terms, p.item, &[], 0)
                    .unwrap_or_else(|e| panic!("{name}, seed {seed}: the setup fails: {e:?}"));
                db.commit(tx).expect("a commit");
                let (ra, rb) = interleave(f, db, a, b, order);
                let q = prepare(f, denial);
                let tx = db.begin().expect("a transaction");
                let left = db
                    .statement(&tx, &q.terms, q.item, &[], 0)
                    .expect("the denial query runs")
                    .result;
                db.abort(tx).expect("an abort");
                (ra, rb, left)
            });
            let ctx = format!("{name}, order {order}, seed {seed}: {ra:?} {rb:?}");
            if order.starts_with("aA") || order.starts_with("bB") {
                assert!(ra.is_ok() != rb.is_ok(), "{ctx}: not exactly one commits");
            } else {
                assert!(ra.is_err() || rb.is_err(), "{ctx}: both commit");
            }
            for e in [&ra, &rb].into_iter().filter_map(|r| r.as_ref().err()) {
                assert!(
                    e.contains("Serialization") || e.contains("Violated"),
                    "{ctx}: a failure other than serialization or the constraint"
                );
            }
            assert_eq!(
                Val::set(BTreeSet::new()),
                left,
                "{ctx}: the constraint is violated"
            );
        }
    }
}

#[test]
fn concurrent_deletes_of_two_identities_cannot_jointly_violate_at_least_one() {
    serial(
        "has_identity",
        &format!("{PEOPLE}{HAS_ONE}"),
        r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert identities.Google { id: identities.key(1), user: users.key(1), gid: "g1" }
  insert identities.Google { id: identities.key(2), user: users.key(1), gid: "g2" }"#,
        "delete i in identities where key i == identities.key(1)",
        "delete i in identities where key i == identities.key(2)",
        "from u in users where not exists i in identities where i.user == key u select u",
    );
}

#[test]
fn concurrent_inserts_cannot_jointly_pass_a_count() {
    let src = format!(
        "{PEOPLE}
constraint identity_limit =
  from u in users
  where count(i for i in identities where i.user == key u) > 2
  select u
"
    );
    serial(
        "identity_limit",
        &src,
        r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert identities.Google { id: identities.key(1), user: users.key(1), gid: "g1" }"#,
        r#"insert identities.Google { id: identities.key(2), user: users.key(1), gid: "g2" }"#,
        r#"insert identities.Google { id: identities.key(3), user: users.key(1), gid: "g3" }"#,
        "from u in users where count(i for i in identities where i.user == key u) > 2 select u",
    );
}

#[test]
fn a_concurrent_delete_and_insert_cannot_jointly_violate_an_inclusion() {
    serial(
        "assigned_member",
        ROSTER,
        r#"insert memberships { id: memberships.key(1), user: "u", org: "o1" }"#,
        r#"delete m in memberships where m.user == "u""#,
        r#"insert assignments { id: assignments.key(1), user: "u", org: "o1", task: "t" }"#,
        "from a in assignments where not exists m in memberships where m.user == a.user and m.org == a.org select a",
    );
}

#[test]
fn a_transition_read_under_negation_is_refused_naming_it() {
    let decl = "constraint once =
  from a in accounts
  where not exists d in inserted(accounts) where key d == key a
  select a";
    assert_eq!(
        (
            "app::once".to_string(),
            "`inserted(app::accounts)` is read under negation".to_string()
        ),
        refused(&format!("{SPEC}\n{decl}\n"))
    );
}

#[test]
fn a_transition_read_inside_an_aggregate_is_refused_naming_it() {
    let decl = "constraint burst =
  from u in users
  where count(a for a in inserted(accounts) where a.owner == key u) > 1
  select u";
    assert_eq!(
        (
            "app::burst".to_string(),
            "`inserted(app::accounts)` is read inside an aggregate".to_string()
        ),
        refused(&format!("{SPEC}\n{decl}\n"))
    );
}

#[test]
fn a_deleted_read_of_a_constructor_under_not_is_refused_naming_its_constructor() {
    let decl = "constraint stays_closed =
  from a in accounts.Open
  where not (exists d in deleted(accounts.Closed) where key d == key a)
  select a";
    assert_eq!(
        (
            "app::stays_closed".to_string(),
            "`deleted(app::accounts.Closed)` is read under negation".to_string()
        ),
        refused(&format!("{SPEC}\n{decl}\n"))
    );
}

#[test]
fn a_transition_read_in_an_if_condition_is_refused() {
    let decl = "constraint quiet =
  from u in users
  where if exists e in inserted(events) where e.body == u.name then false else true
  select u";
    let why = refusals(&format!("{SPEC}\n{decl}\n"));
    assert_eq!(
        vec![(
            "app::quiet".to_string(),
            "`inserted(app::events)` is read under negation".to_string()
        )],
        why
    );
}

#[test]
fn a_migration_declaring_a_transition_read_under_negation_is_refused_naming_the_read() {
    let once = "add constraint once =
  from u in users
  where not exists i in inserted(identities) where i.user == key u
  select u
";
    let got = migrate(&format!("{RUNNING}{once}"), None);
    let Some(Err(CatalogError::Refused(why))) = got.first() else {
        panic!("the migration is not refused: {got:?}")
    };
    assert_eq!(
        vec![
            "constraint auth::once is not admitted: `inserted(auth::identities)` is read under negation",
        ],
        *why
    );
}

const OWNED: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Account = { id: Key accounts, owner: Key users }
relation accounts : Account key id generate sequence
type Note = { id: Key notes, about: Key users on delete cascade }
relation notes : Note key id generate sequence
";

#[test]
fn deleting_a_referenced_row_reads_its_referrers_through_the_implied_index() {
    let setup = r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert users { id: users.key(2), name: "bo" }
  insert users { id: users.key(3), name: "cy" }
  insert accounts { id: accounts.key(1), owner: users.key(1) }
  insert accounts { id: accounts.key(3), owner: users.key(3) }
  insert notes { id: notes.key(2), about: users.key(2) }"#;
    let a = "delete u in users where key u == users.key(2)";
    let b = "transact
  insert accounts from (from u in users where key u == users.key(2)
                        select { id: accounts.key(10), owner: users.key(1) })
  insert notes { id: notes.key(10), about: users.key(3) }";
    for order in ["abAB", "abBA", "aAbB", "baAB", "baBA", "bBaA"] {
        for seed in 0..2 {
            let (ra, rb, notes) = with_engine(seed, OWNED, |f, db| {
                let p = prepare(f, setup);
                let tx = db.begin().expect("a transaction");
                db.statement(&tx, &p.terms, p.item, &[], 0)
                    .unwrap_or_else(|e| panic!("seed {seed}: the setup fails: {e:?}"));
                db.commit(tx).expect("a commit");
                let (ra, rb) = interleave(f, db, a, b, order);
                let q = prepare(f, "from n in notes select n.id");
                let tx = db.begin().expect("a transaction");
                let notes = db
                    .statement(&tx, &q.terms, q.item, &[], 0)
                    .expect("the query runs")
                    .result;
                db.abort(tx).expect("an abort");
                (ra, rb, notes)
            });
            let ctx = format!("order {order}, seed {seed}: {ra:?} {rb:?}");
            assert!(ra.is_ok() && rb.is_ok(), "{ctx}: a probe conflicts");
            assert_eq!(
                Val::set(BTreeSet::from([Val::Int(10)])),
                notes,
                "{ctx}: the cascade"
            );
        }
    }
}
