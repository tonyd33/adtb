use crate::plan::fixture as planfix;

use crate::catalog::{Catalog as Stored, Error as CatalogError};
use crate::storage::Pager;
use crate::write;
use adtb_lang::norm;
use adtb_sim::{Config, Outcome, Sim};

use adtb_testkit::fixture::Fixture;

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
        let pager = Pager::create(rt.clone(), &crate::test_world::options(256)).expect("create");
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
