use super::*;
use crate::parse::{Options, parse_bundle_body};
use crate::schema::Committed;
use crate::signature::{self, Item};

type R<T> = Result<T, Untranslatable>;

const V1: &str = r"module auth
add module auth
add type Tier = Free | Paid
add type User = { id: Key users, name: Text, tier: Tier }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google { gid: Text }
add relation identities : Identity key id generate sequence
";

const RENAMES: &str = "module auth
rename type Tier to Plan
rename constructor Plan.Paid to Pro
rename relation users to people
rename field User.name to display
";

const ADD_SEATS: &str = "module auth\nadd field Tier.Paid.seats: I32 fill 1\n";
const ADD_VERIFIED: &str = "module auth\nadd field Identity.EmailPass.verified: Bool fill false\n";
const ADD_GOLD: &str = "module auth\nadd constructor Tier.Gold\nenable Tier.Gold\n";
const DEPRECATE_FREE: &str = "module auth\ndeprecate Tier.Free\n";
const REMOVE_FREE: &str = "module auth\nremove constructor Tier.Free\n";
const DROP_GID: &str = "module auth\ndrop field Identity.Google.gid\n";
const DEPRECATE_GOOGLE: &str = "module auth\ndeprecate identities.Google\n";
const SPLIT_PAID: &str = "module auth
split constructor Tier.Paid into Monthly | Yearly
  by match
    | Paid -> Monthly
";

fn named(schema: &Schema, name: &str) -> TypeId {
    let i = schema
        .types
        .iter()
        .position(|t| t.name == name && schema.module(t.module).name == "auth")
        .unwrap_or_else(|| panic!("no type {name}"));
    TypeId(i as u32)
}

fn checked(log: &[&str]) -> (Source, Compiled<Schema>) {
    let source = signature::replay(log.iter().copied()).expect("the log replays");
    let compiled = source.check().expect("the version checks");
    (source, compiled)
}

/// Returns the translation of the one query or transaction of `body`, checked at the version
/// the first `pin` migrations of `log` leave, to the version `log` leaves.
fn translated(log: &[&str], pin: usize, body: &str) -> R<NormalQuery> {
    translated_with_hiding(log, pin, body, false)
}

fn translated_with_hiding(log: &[&str], pin: usize, body: &str, hide_new: bool) -> R<NormalQuery> {
    translated_each(log, pin, body, hide_new).remove(0)
}

/// Returns the translation of each query or transaction of `body`, as `translated` does.
fn translated_each(log: &[&str], pin: usize, body: &str, hide_new: bool) -> Vec<R<NormalQuery>> {
    translated_from(&checked(&log[..pin]), log, body, hide_new)
}

/// Returns the translation of each query or transaction of `body`, checked at `pinned`, the
/// version some first migrations of `log` leave, to the version `log` leaves.
fn translated_from(
    pinned: &(Source, Compiled<Schema>),
    log: &[&str],
    body: &str,
    hide_new: bool,
) -> Vec<R<NormalQuery>> {
    translated_to(pinned, &normalised(pinned, body), &checked(log), hide_new)
}

/// Returns each query or transaction of `body`, checked and normalised at `pinned`.
fn normalised(pinned: &(Source, Compiled<Schema>), body: &str) -> Vec<NormalQuery> {
    let (_, from_compiled) = pinned;
    let from = from_compiled.schema();
    let text = format!("import auth\n{body}");
    let (imports, items) = parse_bundle_body(&text, Options::default())
        .ok()
        .expect("the body parses");
    let vis = signature::visible(from, &imports).expect("auth is visible");
    let calls = signature::calls(from_compiled, &vis, &imports, &items)
        .unwrap_or_else(|d| panic!("`{body}` checks at its pin: {}", d.message));
    calls
        .iter()
        .map(|c| {
            match &c.item {
                Item::Query(_, q) => norm::query(from_compiled, q),
                Item::Txn(_, t) => norm::txn(from_compiled, t),
                Item::Fn(..) => panic!("the body holds a query or transaction"),
            }
            .expect("the callable normalises")
        })
        .collect()
}

/// Returns the translation of each of `queries`, normalised at `pinned`, to the version `to`.
fn translated_to(
    pinned: &(Source, Compiled<Schema>),
    queries: &[NormalQuery],
    to: &(Source, Compiled<Schema>),
    hide_new: bool,
) -> Vec<R<NormalQuery>> {
    let (from_source, from) = pinned;
    let (to_source, to) = to;
    let p = Pinned::new(from.schema(), from_source, to, to_source, hide_new);
    queries
        .iter()
        .map(
            |q| match p.as_ref().map_err(Clone::clone).and_then(|p| p.query(q)) {
                Ok(n) => Ok(n),
                Err(Refused::Untranslatable(u)) => Err(u),
                Err(e) => panic!("the translation of a query fails: {e:?}"),
            },
        )
        .collect()
}

/// Returns the operator and reason of the refusal of translating `body`.
#[must_use]
fn refused_across(log: &[&str], pin: usize, body: &str) -> (String, String) {
    match translated(log, pin, body) {
        Ok(_) => panic!("`{body}` translates"),
        Err(u) => (
            u.operator.unwrap_or_else(|| {
                panic!("the refusal of `{body}` names no operator: {}", u.reason)
            }),
            u.reason,
        ),
    }
}

#[test]
fn a_hiding_upsert_keeps_its_visible_scope_distinct_from_its_unique_probe() {
    let add = "module auth\nadd type Note { id: Key notes, slug: Text } = Plain { body: Text } | Other { other: Text }\nadd relation notes : Note key id generate sequence\nadd set Shared of notes = Plain\nadd unique (slug) in notes.Shared\n";
    let grow = "module auth\nadd constructor Note.Link { url: Text }\n  joins notes.Shared\n";
    let enable = "module auth\nenable notes.Link\n";
    let members =
        "module auth\ndrop set Shared of notes\nadd set Shared of notes = Plain | Other\n";
    let body = r#"txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), slug: "same", body: "x" }
      unless conflict on (slug) in Shared else skip
    return 0
"#;
    let mut ran = 0;
    for (name, tail, probe) in [("new constructor", vec![grow, enable], vec![0, 2])] {
        let mut log = vec![V1, add];
        log.extend(tail);
        let (_, current) = checked(&log);
        let current = current.schema();
        for (hide_new, expected) in [(false, probe.clone()), (true, vec![0])] {
            let q = translated_with_hiding(&log, 2, body, hide_new)
                .unwrap_or_else(|e| panic!("{name}: the upsert translates: {e:?}"));
            let scopes: Vec<_> = q
                .terms()
                .iter()
                .filter_map(|n| match n.kind {
                    Kind::Upsert {
                        rel,
                        on: crate::kernel::Conflict::Unique(i),
                        old,
                        ..
                    } => {
                        let Ty::Data { ctors, .. } = &q.terms().var_info(old).ty else {
                            panic!("an upsert's old binding is a declared row");
                        };
                        Some((
                            current.relation(rel).indexes[i as usize].scope.clone(),
                            ctors.clone(),
                        ))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                vec![(Some(probe.clone()), Some(expected))],
                scopes,
                "{name}, hide_new {hide_new}"
            );
            ran += 1;
        }
    }
    assert_eq!(2, ran);
    assert_eq!(
        Err(Untranslatable {
            operator: None,
            reason: "its conflict target is dropped".into(),
        }),
        translated_with_hiding(&[V1, add, members], 2, body, false).map(|_| ())
    );
}

#[test]
fn a_rename_maps_every_object_by_its_catalog_id() {
    let (_, from) = checked(&[V1]);
    let from = from.schema();
    let (_, to) = checked(&[V1, RENAMES]);
    let to = to.schema();
    let map = Map::new(from, to);
    let tier = named(from, "Tier");
    let plan = named(to, "Plan");
    assert_eq!(Some(plan), map.def(tier));
    let users = RelId(
        from.relations
            .iter()
            .position(|r| r.name == "users")
            .expect("users") as u32,
    );
    let people = map.relation(users).expect("users has an image");
    assert_eq!("people", to.relation(people).name);
    assert_eq!(Some(1), map.ctor(tier, 1));
    assert_eq!("Pro", to.ty(plan).ctors()[1].name);
    let user = named(from, "User");
    assert_eq!(
        Some("display".to_string()),
        map.field(user, Some(0), "name")
    );
}

#[test]
fn a_removed_constructor_has_no_image_and_an_added_one_no_preimage() {
    let (_, from) = checked(&[V1, ADD_GOLD]);
    let from = from.schema();
    let (_, to) = checked(&[V1, ADD_GOLD, DEPRECATE_FREE, REMOVE_FREE]);
    let to = to.schema();
    let map = Map::new(from, to);
    let tier = named(from, "Tier");
    assert_eq!(None, map.ctor(tier, 0));
    assert_eq!(Some(0), map.ctor(tier, 1));
    assert_eq!(Some(1), map.ctor_back(tier, 0));
    let (_, pin) = checked(&[V1]);
    let pin = pin.schema();
    let (_, gold) = checked(&[V1, ADD_GOLD]);
    let gold = gold.schema();
    let added = Map::new(pin, gold);
    assert_eq!(None, added.ctor_back(tier, 2));
}

#[test]
fn every_pin_narrows_rows_to_its_constructors_and_keeps_keys_whole() {
    const ADD_APPLE: &str = "module auth\nadd constructor Identity.Apple { sub: Text }\n";
    let (_, from) = checked(&[V1]);
    let from = from.schema();
    let (_, to) = checked(&[V1, ADD_APPLE]);
    let to = to.schema();
    let map = Map::new(from, to);
    let identities = RelId(
        from.relations
            .iter()
            .position(|r| r.name == "identities")
            .expect("identities") as u32,
    );
    let identity = named(from, "Identity");
    let key = Ty::Key {
        rel: RelRef::Declared(identities),
        ctors: KeyCtors::All,
    };
    let row = Ty::Data {
        def: identity,
        args: Vec::new(),
        ctors: None,
    };
    let tier = Ty::Data {
        def: named(from, "Tier"),
        args: Vec::new(),
        ctors: None,
    };
    assert_eq!(Ok(key.clone()), map.ty(&key));
    assert_eq!(Ok(tier.clone()), map.ty(&tier));
    assert_eq!(
        Ok(Ty::Data {
            def: identity,
            args: Vec::new(),
            ctors: Some(vec![0, 1])
        }),
        map.ty(&row)
    );
}

#[test]
fn a_key_whose_selection_resolves_at_its_migration_end_is_lost() {
    let (_, from) = checked(&[V1]);
    let from = from.schema();
    let map = Map::new(from, from);
    let identities = RelId(
        from.relations
            .iter()
            .position(|r| r.name == "identities")
            .expect("identities") as u32,
    );
    let late = Ty::Key {
        rel: RelRef::Declared(identities),
        ctors: KeyCtors::Late("Settled".to_string()),
    };
    assert_eq!(
        Err(Loss::Unlinked("identities.Settled".to_string())),
        map.ty(&Ty::Set(Box::new(late)))
    );
}

#[test]
fn a_key_of_a_later_relation_whose_selection_resolves_at_its_migration_end_is_lost() {
    let file = crate::parse::parse_schema(
        "module m\ntype O = { id: Key os, u: Key later.Gold }\nrelation os : O key id generate sequence",
        Options::default(),
    )
    .ok()
    .expect("parses");
    let ahead = crate::schema::Ahead::new(
        "m".to_string(),
        "later".to_string(),
        true,
        crate::schema::AheadState::Pending,
        Adder::Operator(0),
    );
    let from =
        crate::schema::build_ahead(&[(crate::span::FileId(0), file)], &[ahead]).expect("builds");
    let os = from.relation(RelId(
        from.relations
            .iter()
            .position(|r| r.name == "os")
            .expect("os") as u32,
    ));
    let u = from
        .ty(os.elem_def)
        .field_named(0, "u")
        .expect("u")
        .ty
        .clone();
    let map = Map::over(&from, &from, vec![Later::At(RelId(0))]);
    assert_eq!(Err(Loss::Unlinked("later.Gold".to_string())), map.ty(&u));
}

const SESSIONS: &str = "module auth\nadd type Session = { id: Key sessions, who: Key identities }\n\
                        add relation sessions : Session key id generate sequence\n";
const ADD_APPLE_ENABLED: &str =
    "module auth\nadd constructor Identity.Apple { sub: Text }\nenable identities.Apple\n";

/// Returns each node of `q` whose kind `pick` keeps, with its type.
fn nodes<T>(q: &NormalQuery, mut pick: impl FnMut(&Kind) -> Option<T>) -> Vec<(T, Ty)> {
    q.terms()
        .iter()
        .filter_map(|n| pick(&n.kind).map(|x| (x, n.ty.clone())))
        .collect()
}

#[test]
fn a_hiding_pin_dereferences_a_whole_key_admitting_the_pinned_constructors_only() {
    let body = "query q () -> Set (Key users) = from s in sessions select s.who.user\n";
    let log = [V1, SESSIONS, ADD_APPLE_ENABLED];
    let (_, to) = checked(&log);
    let to = to.schema();
    let identities = RelId(
        to.relations
            .iter()
            .position(|r| r.name == "identities")
            .expect("identities") as u32,
    );
    let identity = to.relation(identities).elem_def;
    let whole_key = Ty::Key {
        rel: RelRef::Declared(identities),
        ctors: KeyCtors::All,
    };
    for (hide_new, admits) in [(true, Some(vec![0, 1])), (false, None)] {
        let q = translated_with_hiding(&log, 2, body, hide_new)
            .unwrap_or_else(|e| panic!("hide_new {hide_new}: it translates: {e:?}"));
        let derefs = nodes(&q, |k| match k {
            Kind::Deref { key, admits, .. } => Some((*key, admits.clone())),
            _ => None,
        });
        let [((key, got), ty)] = derefs.as_slice() else {
            panic!("hide_new {hide_new}: one dereference, not {derefs:?}");
        };
        let row = Ty::Data {
            def: identity,
            args: Vec::new(),
            ctors: admits.clone(),
        };
        assert_eq!(
            (&whole_key, &admits, &row),
            (q.terms().ty(*key), got, ty),
            "hide_new {hide_new}"
        );
        let t = q
            .terms()
            .iter()
            .position(|n| matches!(n.kind, Kind::Deref { .. }))
            .expect("the dereference");
        let total = crate::analysis::Totality::new(to, crate::capability::RootKind::Query)
            .total(q.terms(), Tm(t as u32));
        assert_eq!(!hide_new, total, "hide_new {hide_new}");
    }
}

#[test]
fn a_hiding_pin_dereference_requires_reading_the_pinned_constructors_only() {
    use crate::permission::{Requirement, Target};
    let body = "query q () -> Set (Key users) = from s in sessions select s.who.user\n";
    let log = [V1, SESSIONS, ADD_APPLE_ENABLED];
    let (_, to_compiled) = checked(&log);
    let to = Committed::new(to_compiled.schema().clone()).expect("a replayed log settles");
    let q = translated_with_hiding(&log, 2, body, true).expect("it translates");
    let program = norm::program(&to_compiled).expect("the program normalises");
    let got: BTreeSet<Requirement> =
        crate::permission::normal(&to, &program, q.terms(), q.body(), q.params())
            .expect("requirements analyze")
            .iter()
            .cloned()
            .collect();
    let reads = |relation: &str, ctor: &str, fields: &[&str]| {
        let r = RelId(
            to.relations
                .iter()
                .position(|r| {
                    r.name == relation && to.module(to.ty(r.elem_def).module).name == "auth"
                })
                .expect("relation") as u32,
        );
        let td = to.ty(to.relation(r).elem_def);
        let c = td.ctor_named(ctor).expect("constructor");
        let t = Target {
            relation: to.ids().relation_id(r).expect("catalogued"),
            constructor: to.ids().ctor_id(c).expect("catalogued"),
        };
        std::iter::once(Requirement::ReadConstructor(t))
            .chain(fields.iter().map(|f| {
                let f = td.field(Some(c), f).expect("field");
                Requirement::ReadField(t, to.ids().field_id(f).expect("catalogued"))
            }))
            .collect::<Vec<_>>()
    };
    let want: BTreeSet<Requirement> = [
        reads("sessions", "Session", &["id", "who"]),
        reads("identities", "EmailPass", &["id", "user"]),
        reads("identities", "Google", &["id", "user"]),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(want, got);
}

#[test]
fn a_field_of_a_sum_that_gained_a_constructor_keeps_its_declared_type_and_matches_it_unreachable() {
    let body =
        "query q () -> Set I64 = from u in users select match u.tier | Free -> 0 | Paid -> 1\n";
    let (_, to) = checked(&[V1, ADD_GOLD]);
    let to = to.schema();
    let q = translated(&[V1, ADD_GOLD], 1, body).expect("it translates");
    let tier = Ty::Data {
        def: named(to, "Tier"),
        args: Vec::new(),
        ctors: None,
    };
    let fields = nodes(&q, |k| match k {
        Kind::Field(_, l) if l == "tier" => Some(()),
        _ => None,
    });
    assert_eq!(vec![((), tier)], fields);
    let arms = nodes(&q, |k| match k {
        Kind::Case { arms, .. } => Some(
            arms.iter()
                .map(|(c, _, b)| (*c, matches!(&q.terms().kind(*b), Kind::Unreachable)))
                .collect::<Vec<_>>(),
        ),
        _ => None,
    });
    let arms: Vec<_> = arms.into_iter().map(|(a, _)| a).collect();
    assert_eq!(vec![vec![(0, false), (1, false), (2, true)]], arms);
}

#[test]
fn a_field_of_a_removed_constructor_is_a_match_with_no_arm() {
    let log = [
        V1,
        DEPRECATE_GOOGLE,
        "module auth\nremove constructor Identity.Google\n",
    ];
    let body = "query q () -> Set Text = from g in identities.Google select g.gid\n";
    let q = translated(&log, 1, body).expect("it translates");
    let gid = nodes(&q, |k| match k {
        Kind::Field(_, l) if l == "gid" => Some(()),
        _ => None,
    });
    assert_eq!(Vec::<((), Ty)>::new(), gid);
    let text = Ty::Base(Base::Text);
    let cases = nodes(&q, |k| match k {
        Kind::Case { arms, .. } => Some(arms.len()),
        _ => None,
    });
    assert_eq!(vec![(0, text)], cases);
}

#[test]
fn an_arm_over_a_conflicting_row_binds_its_constructor_only() {
    let notes = "module auth\nadd type Note { id: Key notes } = Plain { body: Text }\n\
                 add relation notes : Note key id generate sequence\n";
    let link = "module auth\nadd constructor Note.Link { url: Text }\nenable notes.Link\n";
    let body = r#"txn put () -> I64 =
  transact
    insert notes.Plain { id: notes.key(1), body: "x" }
      unless conflict on key else update old set Plain { old with body: match old
        | Plain p -> p.body }
    return 0
"#;
    let log = [V1, notes, link];
    let (_, to) = checked(&log);
    let to = to.schema();
    let q = translated(&log, 2, body).expect("it translates");
    let note = named(to, "Note");
    let bound: Vec<Ty> = q
        .terms()
        .iter()
        .filter_map(|n| match &n.kind {
            Kind::Case { arms, .. } => Some(arms.clone()),
            _ => None,
        })
        .flatten()
        .filter(|(_, _, b)| !matches!(q.terms().kind(*b), Kind::Unreachable))
        .map(|(_, y, _)| q.terms().var_info(y).ty.clone())
        .collect();
    let plain = Ty::Data {
        def: note,
        args: Vec::new(),
        ctors: Some(vec![0]),
    };
    assert!(!bound.is_empty(), "a matched arm");
    assert_eq!(vec![plain; bound.len()], bound);
}

#[test]
fn a_dropped_field_has_no_image() {
    let (_, from) = checked(&[V1]);
    let from = from.schema();
    let (_, to) = checked(&[V1, DROP_GID]);
    let to = to.schema();
    let map = Map::new(from, to);
    let identity = named(from, "Identity");
    assert_eq!(None, map.field(identity, Some(1), "gid"));
    assert_eq!(
        Some("email".to_string()),
        map.field(identity, Some(0), "email")
    );
}

#[test]
fn a_read_of_a_dropped_field_is_refused_naming_the_drop() {
    let (op, reason) = refused_across(
        &[V1, DROP_GID],
        1,
        "query q () -> Set Text = from i in identities.Google select i.gid",
    );
    assert_eq!("drop field Identity.Google.gid", op);
    assert_eq!(
        "it reads field `gid` of `Identity`, which the current version lacks",
        reason
    );
    translated(
        &[V1, DROP_GID],
        1,
        "query q () -> Set Text = from i in identities.EmailPass select i.email",
    )
    .expect("a query that does not read the field translates");
}

#[test]
fn a_count_of_values_that_gained_a_field_is_refused_and_a_count_of_rows_translates() {
    let (op, reason) = refused_across(
        &[V1, ADD_SEATS],
        1,
        "query q () -> I64 = count(t for t in (from u in users select u.tier))",
    );
    assert_eq!("add field Tier.Paid.seats: I32 fill 1", op);
    assert_eq!(
        "it counts the values of `Tier` whole, and its field `seats` differs",
        reason
    );
    translated(
        &[V1, ADD_SEATS],
        1,
        "query q () -> I64 = count(u for u in users)",
    )
    .expect("a count of rows translates");
}

#[test]
fn a_comparison_of_values_that_gained_a_field_is_refused() {
    let (op, reason) = refused_across(
        &[V1, ADD_SEATS],
        1,
        "query q () -> Set Text = from u in users where u.tier == Paid select u.name",
    );
    assert_eq!("add field Tier.Paid.seats: I32 fill 1", op);
    assert_eq!(
        "it compares `Tier` whole, and its field `seats` differs",
        reason
    );
}

#[test]
fn a_parameter_or_comparison_reaching_a_removed_constructor_is_refused() {
    let log = [V1, ADD_GOLD, DEPRECATE_FREE, REMOVE_FREE];
    for (body, want) in [
        (
            "query q (t: Tier) -> Set Text = from u in users select u.name",
            "it takes a parameter of `Tier.Free`, which the current version lacks",
        ),
        (
            "query q () -> Set Text = from u in users where u.tier == Gold select u.name",
            "it compares `Tier.Free`, which the current version lacks",
        ),
    ] {
        let (op, reason) = refused_across(&log, 2, body);
        assert_eq!("remove constructor Tier.Free", op, "{body}");
        assert_eq!(want, reason, "{body}");
    }
}

#[test]
fn a_whole_row_holding_a_split_type_is_refused_naming_the_split() {
    let (op, _) = refused_across(
        &[V1, SPLIT_PAID],
        1,
        "query q () -> Set User = from u in users select u",
    );
    assert!(op.starts_with("split constructor Tier.Paid"), "{op}");
    translated(
        &[V1, SPLIT_PAID],
        1,
        "query q () -> Set Text = from i in identities.EmailPass select i.email",
    )
    .expect("a query that does not reach the type translates");
}

#[test]
fn a_write_into_a_deprecated_constructor_is_refused_naming_the_deprecation() {
    let body = "txn t (g: Text) -> Text =
  transact
    insert identities.Google { user: users.key(1), gid: g }
    return g";
    let (op, reason) = refused_across(&[V1, DEPRECATE_GOOGLE], 1, body);
    assert_eq!("deprecate identities.Google", op);
    assert_eq!(
        "it writes `Identity.Google`, which is deprecated now",
        reason
    );
}

#[test]
fn a_write_of_a_constructor_that_gained_a_field_translates_and_a_nested_one_is_refused() {
    let insert = "txn t (e: Text) -> Text =
  transact
    insert identities.EmailPass { user: users.key(1), email: e }
    return e";
    translated(&[V1, ADD_VERIFIED], 1, insert).expect("the insert takes the fill");
    let nested = "txn t (n: Text) -> Text =
  transact
    insert users { name: n, tier: Paid }
    return n";
    let (op, reason) = refused_across(&[V1, ADD_SEATS], 1, nested);
    assert_eq!("add field Tier.Paid.seats: I32 fill 1", op);
    assert_eq!(
        "it builds `Tier.Paid` without field `seats`, outside a write",
        reason
    );
}

#[test]
fn a_parameter_reaching_a_type_under_two_narrowings_checks_both() {
    let body = "query q (p: { a: Identity.Google, b: Identity.EmailPass }) -> Set Text =
  from i in identities.EmailPass select i.email";
    let (op, reason) = refused_across(&[V1, ADD_VERIFIED], 1, body);
    assert_eq!("add field Identity.EmailPass.verified: Bool fill false", op);
    assert_eq!(
        "it takes a parameter of `Identity` whole, and its field `verified` differs",
        reason
    );
}

const SHOP: &str = r"module auth
add module auth
add type Price = { sku: Text, cents: I64 }
add relation prices : Price key sku
add type Order = { id: Key orders, quoted: Price }
add relation orders : Order key id generate sequence
";
const ADD_CURRENCY: &str = "module auth\nadd field Price.currency: Text fill \"USD\"\n";

#[test]
fn a_count_of_rows_read_out_of_a_field_is_refused_and_one_of_filtered_rows_translates() {
    let log = [SHOP, ADD_CURRENCY];
    for body in [
        "query n () -> I64 = count(p for p in (from o in orders select o.quoted))",
        "query n () -> I64 = count(p for p in (from o in orders, p in [o.quoted] select p))",
    ] {
        let (op, reason) = refused_across(&log, 1, body);
        assert_eq!("add field Price.currency: Text fill \"USD\"", op, "{body}");
        assert_eq!(
            "it counts the values of `Price` whole, and its field `currency` differs", reason,
            "{body}"
        );
    }
    translated(
        &log,
        1,
        "query n () -> I64 = count(p for p in (from p in prices where p.cents > 0 select p))",
    )
    .expect("a count of filtered rows translates");
}

#[test]
fn a_removed_constructor_is_blamed_on_its_removal_not_a_set_listing_it() {
    let log = [
        V1,
        "module auth\nadd set Social of identities = Google | EmailPass\n",
        "module auth\ndeprecate identities.Google\n",
        "module auth\nremove constructor Identity.Google\n",
    ];
    let body = "txn t (g: Text) -> Text =
  transact
    insert identities.Google { user: users.key(1), gid: g }
    return g";
    let (op, reason) = refused_across(&log, 1, body);
    assert_eq!("remove constructor Identity.Google", op);
    assert_eq!(
        "it writes `Identity.Google`, which the current version lacks",
        reason
    );
}

#[test]
fn a_read_of_the_past_or_of_a_file_is_refused_naming_no_operator() {
    let at = "query q (t: Instant) -> Set Text = at instant t { from u in users select u.name }";
    assert_eq!(
        Err(Untranslatable {
            operator: None,
            reason: "it reads the past in an `at` block".into(),
        }),
        translated(&[V1, ADD_SEATS], 1, at).map(|_| ())
    );
    let file = r#"txn q () -> I64 =
  transact
    insert users from read_csv("users.csv", User)
    return 0"#;
    assert_eq!(
        Err(Untranslatable {
            operator: None,
            reason: "it reads a file".into(),
        }),
        translated(&[V1, ADD_SEATS], 1, file).map(|_| ())
    );
}

#[test]
fn a_comprehension_yielding_its_row_under_shared_branches_is_walked_once() {
    let mut terms = Terms::default();
    let c = terms.var("c", Ty::Unit);
    let test = terms.add_ill_typed(Kind::Var(c), Ty::Unit);
    let row = terms.var("x", Ty::Unit);
    let element = terms.add_ill_typed(Kind::Var(row), Ty::Unit);
    let yielded = terms.add_ill_typed(Kind::Singleton(element), Ty::Unit);
    let set = terms.doubled(test, yielded, 64);
    assert!(forms::yields(&terms, set, row));
}

#[test]
fn a_wasm_function_taken_to_another_module_conflicts_with_a_pin_of_the_old_one() {
    let wasm = |m: &str, op: &str| {
        format!("module auth\n{op} fn f (t: Text) -> Text = wasm \"{m}\" export \"e\"\n")
    };
    let (define, redefine) = (wasm("m", "define"), wasm("n", "redefine"));
    let (at_pin, _) = checked(&[V1, &define]);
    let (next, _) = checked(&[V1, &define, &redefine]);
    assert_eq!(
        Some((
            "redefine fn f (t: Text) -> Text = wasm \"n\" export \"e\"".to_string(),
            "f".to_string()
        )),
        host_conflict(&at_pin, &at_pin, &next).expect("the functions resolve")
    );
}

#[test]
fn a_rename_rewriting_a_language_body_is_no_change_of_a_host_function_at_the_pin() {
    let host = "module auth\ndefine fn f (t: Text) -> Text = host \"a\" version \"1.0\" deterministic\nadd type H { id: Key hs } = R { n: Text }\nadd relation hs : H key id generate sequence\n";
    let reader = "module auth\nredefine fn f (t: Text) -> Text = if count(h for h in hs) == 0 then t else t\n";
    let rename = "module auth\nrename relation hs to hz\n";
    let (at_pin, _) = checked(&[V1, host]);
    let (before, _) = checked(&[V1, host, reader]);
    let (next, _) = checked(&[V1, host, reader, rename]);
    let body = |s: &Source| {
        let f = s.functions().expect("the functions resolve").remove(0);
        assert_eq!("f", f.name);
        f.code
    };
    assert_ne!(body(&before), body(&next));
    assert_eq!(
        None,
        host_conflict(&before, &at_pin, &next).expect("the functions resolve")
    );
}

/// Each composite key's fields, and each rename of one field `f` to `g` as `fg`, marked `!`
/// where it reorders the key's fields by name.
const KEY_RENAMES: [(&str, &str); 2] = [
    ("b d", "ba bc be! da! dc de"),
    ("b d f", "ba bc be! bg! da! dc de dg! fa! fc! fe fg"),
];

#[test]
fn a_composite_key_compared_ordered_is_refused_across_a_rename_that_reorders_its_fields() {
    let mut reached = (0, 0);
    for (fields, renames) in KEY_RENAMES {
        let fields: Vec<&str> = fields.split(' ').collect();
        let decl: Vec<String> = fields.iter().map(|f| format!("{f}: Text")).collect();
        let schema = format!(
            "module auth\nadd type P = {{ {} }}\nadd relation p : P key ({})\n\
             add type R = {{ o: Key p, n: Text }}\nadd relation r : R key (o, n)\n",
            decl.join(", "),
            fields.join(", ")
        );
        let args: Vec<String> = fields.iter().map(|f| format!("{f}: \"9\"")).collect();
        let key = format!("p.key({})", args.join(", "));
        let body = format!(
            "query lt () -> Set Text = from x in p where key x < {key} select \"below\"\n\
             query first () -> Set (Key p) = from x in p select key x order by key x take 1\n\
             query nested () -> Set Text = from x in r where key x < r.key(o: {key}, n: \"z\") select x.n\n"
        );
        for rename in renames.split(' ') {
            let op = format!("rename field P.{} to {}", &rename[..1], &rename[1..2]);
            let later = format!("module auth\n{op}\n");
            let got = translated_each(&[V1, &schema, &later], 2, &body, false);
            for (q, got) in ["lt", "first", "nested"].iter().zip(got) {
                let case = format!("`{q}` across `{op}` of {fields:?}");
                match got {
                    Ok(_) if !rename.ends_with('!') => reached.0 += 1,
                    Err(u) if rename.ends_with('!') => {
                        let why = "it compares `Key p` ordered, and its order differs";
                        assert_eq!(
                            (Some(op.clone()), why.into()),
                            (u.operator, u.reason),
                            "{case}"
                        );
                        reached.1 += 1;
                    }
                    other => panic!("{case}: {other:?}"),
                }
            }
        }
    }
    assert_eq!((30, 24), reached);
}

#[test]
fn a_pinned_key_is_judged_by_its_fields_and_its_identity_across_each_operator() {
    use adtb_testkit::sections::{Section, expectations, sections};
    let read = |f: &str| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pinned-keys/");
        std::fs::read_to_string(format!("{dir}{f}")).expect("the search is readable")
    };
    let sections = sections(&read("search.txt"));
    let expected = expectations(&read("translations.txt"));
    let of = |kind: &str, rel: &str| -> Vec<&Section> {
        sections
            .iter()
            .filter(|s| s.kind == kind && s.name[0] == rel)
            .collect()
    };
    let mut got = BTreeMap::new();
    for r in sections.iter().filter(|s| s.kind == "relation") {
        let rel = &r.name[0];
        let bodies = of("body", rel);
        let text: String = bodies.iter().map(|b| b.text.as_str()).collect();
        for op in of("operator", rel) {
            let each = translated_each(&[&r.text, &op.text], 1, &text, false);
            for (b, got_one) in bodies.iter().zip(each) {
                let answer = match got_one {
                    Ok(_) => "translates".to_owned(),
                    Err(u) => format!("refused {:?} {:?}", u.operator, u.reason),
                };
                got.insert(format!("{rel} {} {}", op.name[1], b.name[1]), answer);
            }
        }
    }
    assert_eq!(84, got.len(), "every relation, operator and body pair ran");
    let differs: Vec<String> = got
        .iter()
        .filter(|(k, v)| expected.get(*k) != Some(v))
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    assert!(
        differs.is_empty(),
        "translations differ:\n{}",
        differs.join("\n")
    );
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>(),
        "every expectation is reached"
    );
}

/// Returns the migrations of `text`, each a `module auth` line and the operators after it.
fn migrations(text: &str) -> Vec<String> {
    text.split("module auth\n")
        .filter(|m| !m.is_empty())
        .map(|m| format!("module auth\n{m}\n"))
        .collect()
}

/// Every body pinned after an operator translates across the revert of that operator as across
/// its inverse written as migrations. Across a stand-in an `oracle` part marks `refusal`, it is
/// only refused.
#[test]
fn a_revert_is_judged_as_its_inverse_written_out_or_a_marked_stand_in() {
    use adtb_testkit::sections::{Section, expectations, sections};
    let read = |f: &str| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pinned-revert/");
        std::fs::read_to_string(format!("{dir}{f}")).expect("the search is readable")
    };
    let sections = sections(&read("search.txt"));
    let expected = expectations(&read("translations.txt"));
    let base = sections
        .iter()
        .find(|s| s.kind == "base")
        .expect("the search has a base");
    let mut got = BTreeMap::new();
    let mut refusals = 0;
    let mut refusal_only = BTreeSet::new();
    for op in sections.iter().filter(|s| s.kind == "operator") {
        let name = &op.name[0];
        let bodies: Vec<&Section> = sections
            .iter()
            .filter(|s| s.kind == "body" && s.name[0] == *name)
            .collect();
        let text: String = bodies.iter().map(|b| b.text.as_str()).collect();
        let mut pin = vec![base.text.clone()];
        pin.extend(migrations(&op.text));
        let reverted = format!("revert {:04}", pin.len());
        let revert = match op.part("revert") {
            "" => format!("module auth\n{reverted}"),
            given => given.to_owned(),
        };
        let pinned = checked(&pin.iter().map(String::as_str).collect::<Vec<_>>());
        for (side, then, blamed) in [
            ("inverse", op.part("inverse"), op.part("blame")),
            ("revert", revert.as_str(), reverted.as_str()),
        ] {
            let mut log = pin.clone();
            log.extend(migrations(then));
            let log: Vec<&str> = log.iter().map(String::as_str).collect();
            for (b, one) in bodies
                .iter()
                .zip(translated_from(&pinned, &log, &text, false))
            {
                let pair = format!("{name} {} {side}", b.name[1]);
                let answer = match one {
                    Ok(_) => "translates".to_owned(),
                    Err(u) => {
                        assert_eq!(Some(blamed), u.operator.as_deref(), "{pair}");
                        refusals += 1;
                        format!("refused {:?}", u.reason)
                    }
                };
                if side == "inverse" && op.part("oracle") == "refusal" {
                    refusal_only.insert(pair.clone());
                }
                got.insert(pair, answer);
            }
        }
    }
    let differs: Vec<String> = got
        .iter()
        .filter(|(k, v)| {
            let pair = k.rsplit_once(' ').expect("a side ends the key").0;
            match expected.get(pair) {
                Some(e) if refusal_only.contains(*k) => {
                    !(e.starts_with("refused") && v.starts_with("refused"))
                }
                e => e != Some(*v),
            }
        })
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    assert!(
        differs.is_empty(),
        "translations differ:\n{}",
        differs.join("\n")
    );
    let reached: Vec<String> = expected
        .keys()
        .flat_map(|k| [format!("{k} inverse"), format!("{k} revert")])
        .collect();
    assert_eq!(
        reached,
        got.keys().cloned().collect::<Vec<_>>(),
        "every pair is reached on both sides"
    );
    assert_eq!(
        (16, 20, 1),
        (expected.len(), refusals, refusal_only.len()),
        "every operator and body pair ran, each refusal on both sides, and each stand-in"
    );
}

#[test]
fn a_field_moved_down_has_an_image_only_in_the_constructor_it_now_sits_in() {
    let add = "module auth\nadd field Identity.note: Text fill \"n\"\n";
    let down = "module auth\nmove field Identity.note to Identity.EmailPass\n";
    let (_, from) = checked(&[V1, add]);
    let from = from.schema();
    let (_, to) = checked(&[V1, add, down]);
    let to = to.schema();
    let map = Map::new(from, to);
    let def = named(from, "Identity");
    let image = |c, f| map.field(def, c, f);
    assert_eq!(
        (Some("note".to_owned()), None, None),
        (
            image(Some(0), "note"),
            image(Some(1), "note"),
            image(None, "note")
        )
    );
    assert_eq!(
        (Some("user".to_owned()), Some("user".to_owned())),
        (image(Some(1), "user"), image(None, "user"))
    );
}

const AHEAD_MIG: &str = "module auth
add field User.l: Option (Key later) fill None
add type L = { id: Key later }
add relation later : L key id generate sequence
";

const INSERT_USER: &str = "txn t (n: Text) -> Text =
  transact
    insert users { name: n, tier: Free }
    return n";

#[test]
fn an_insert_takes_a_fill_whose_type_holds_a_key_of_a_relation_a_later_operator_adds() {
    let q = translated(&[V1, AHEAD_MIG], 1, INSERT_USER).expect("the insert takes the fill");
    let (_, to) = checked(&[V1, AHEAD_MIG]);
    let to = to.schema();
    let later = to
        .relations
        .iter()
        .position(|r| r.name == "later")
        .map(|i| RelRef::Declared(RelId(i as u32)))
        .expect("`later` is declared");
    let keys: BTreeSet<RelRef> = q
        .terms()
        .iter()
        .filter_map(|n| match &n.ty {
            Ty::Data { args, .. } => args.iter().find_map(|a| match a {
                Ty::Key { rel, .. } => Some(*rel),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(BTreeSet::from([later]), keys);
}

#[test]
fn a_fill_holding_a_key_of_a_relation_added_later_and_dropped_since_is_refused() {
    let m2 = "module auth
define fn isnone (x: Option (Key later)) -> Bool = x == None
add field User.m: Bool fill isnone(None)
add type L = { id: Key later }
add relation later : L key id generate sequence
";
    let m3 = "module auth\ndrop fn isnone\ndrop relation later\ndrop type L\n";
    assert_eq!(
        Err(Untranslatable {
            operator: Some("add field User.m: Bool fill isnone(None)".to_string()),
            reason: "it reaches `later`, which the current version lacks".to_string(),
        }),
        translated(&[V1, m2, m3], 1, INSERT_USER).map(drop)
    );
}

/// A relation of the model: the migration that added it, `None` for one of the pin, its name,
/// and whether a revert put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    added: Option<usize>,
    name: String,
    put_back: bool,
}

/// What the model holds after a migration: each slot's relation, and the first operator that
/// dropped the relation of the pin in each slot.
#[derive(Clone, Debug, Default)]
struct Model {
    slots: [Option<Held>; 2],
    dropped: [Option<String>; 2],
}

const SLOTS: [(&str, &str); 2] = [("xs", "X"), ("ys", "Y")];

const PIN: &str = "module auth
add module auth
add type X = { id: Key xs, n: I64 }
add relation xs : X key id generate sequence
add type Y = { id: Key ys, n: I64 }
add relation ys : Y key id generate sequence
";

/// Returns the operators of `op` on slot `s` of `m`, and applies them to `m` as migration `at`.
fn step(m: &mut Model, s: usize, op: u8, at: usize) -> Option<Vec<String>> {
    let (base, ty) = SLOTS[s];
    match (op, m.slots[s].clone()) {
        (0, None) => {
            m.slots[s] = Some(Held {
                added: Some(at),
                name: base.to_string(),
                put_back: false,
            });
            Some(vec![
                format!("add type {ty} = {{ id: Key {base}, n: I64 }}"),
                format!("add relation {base} : {ty} key id generate sequence"),
            ])
        }
        (1, Some(h)) => {
            let drop = format!("drop relation {}", h.name);
            if h.added.is_none() && m.dropped[s].is_none() {
                m.dropped[s] = Some(drop.clone());
            }
            m.slots[s] = None;
            Some(vec![drop, format!("drop type {ty}")])
        }
        (2, Some(h)) => {
            let to = if h.name == base {
                format!("{base}2")
            } else {
                base.to_string()
            };
            let rename = format!("rename relation {} to {to}", h.name);
            m.slots[s] = Some(Held { name: to, ..h });
            Some(vec![rename])
        }
        _ => None,
    }
}

/// Returns each log the model generates: the pin, then up to `n` migrations, each one operator
/// or two on the two slots, or the revert of the migration before it when that is no revert.
/// With each log, the model after it. A relation a revert puts back is a new relation.
fn generated(n: usize) -> Vec<(Vec<String>, Model)> {
    let start = Model {
        slots: SLOTS.map(|(name, _)| {
            Some(Held {
                added: None,
                name: name.to_string(),
                put_back: false,
            })
        }),
        dropped: [None, None],
    };
    let mut out = Vec::new();
    let mut frontier = vec![(vec![PIN.to_string()], vec![start], false)];
    for _ in 0..n {
        let mut next = Vec::new();
        for (log, models, reverted) in &frontier {
            let at = log.len();
            let last = models.last().expect("a model per migration");
            let mut grow = |text: String, m: Model, reverted: bool| {
                let mut log = log.clone();
                log.push(text);
                let mut models = models.clone();
                models.push(m.clone());
                out.push((log.clone(), m));
                next.push((log, models, reverted));
            };
            if at > 1 && !reverted {
                let mut m = models[at - 2].clone();
                m.dropped = last.dropped.clone();
                for (slot, now) in m.slots.iter_mut().zip(&last.slots) {
                    if let Some(h) = slot
                        && now.as_ref().map(|n| n.added) != Some(h.added)
                    {
                        h.added = Some(at);
                        h.put_back = true;
                    }
                }
                grow(format!("module auth\nrevert {at:04}\n"), m, true);
            }
            for ops in [(Some(0), None), (Some(1), None), (Some(2), None)]
                .into_iter()
                .chain([0, 1, 2].map(|o| (None, Some(o))))
                .chain((0..3).flat_map(|a| (0..3).map(move |b| (Some(a), Some(b)))))
            {
                let mut m = last.clone();
                let mut text = vec!["module auth".to_string()];
                let applied = [ops.0, ops.1].into_iter().enumerate().all(|(s, op)| {
                    let Some(op) = op else { return true };
                    step(&mut m, s, op, at).map(|t| text.extend(t)).is_some()
                });
                if applied {
                    grow(text.join("\n") + "\n", m, false);
                }
            }
        }
        frontier = next;
    }
    out
}

type Outcome = Result<Option<CatRelId>, (Option<String>, String)>;

/// Returns the history that renames the pin's `xs` and adds a new `xs`, with its model.
fn renamed_then_added() -> (Vec<String>, Model) {
    let held = |name: &str| {
        Some(Held {
            added: None,
            name: name.to_string(),
            put_back: false,
        })
    };
    let rename = "module auth
rename relation xs to xs2
add type X3 = { id: Key xs, n: I64 }
add relation xs : X3 key id generate sequence
";
    (
        vec![PIN.to_string(), rename.to_string()],
        Model {
            slots: [held("xs2"), held("ys")],
            dropped: [None, None],
        },
    )
}

/// Returns the translation of each of `transactions`, normalised at `pinned`, to the version
/// `to`: the catalog id of the relation its one insert targets, or its refusal.
fn inserted(
    pinned: &(Source, Compiled<Schema>),
    transactions: &[NormalQuery],
    to: &(Source, Compiled<Schema>),
) -> Vec<Outcome> {
    translated_to(pinned, transactions, to, false)
        .into_iter()
        .map(|r| {
            r.map(|q| {
                let inserted: Vec<RelId> = q
                    .terms()
                    .iter()
                    .filter_map(|n| match n.kind {
                        Kind::Insert(rel, _) => Some(rel),
                        _ => None,
                    })
                    .collect();
                let [rel] = inserted.as_slice() else {
                    panic!("one insert, not {inserted:?}");
                };
                to.1.schema().ids().relation_id(*rel)
            })
            .map_err(|u| (u.operator, u.reason))
        })
        .collect()
}

/// Returns the kinds of history `migrations`, after the pin, with `model` after them, is.
fn kinds<'m>(migrations: &'m [&str], model: &'m Model) -> impl Iterator<Item = &'static str> + 'm {
    let any = move |p: &dyn Fn(&str) -> bool| migrations.iter().any(|t| p(t));
    [
        ("add", any(&|t| t.contains("add relation"))),
        ("drop", any(&|t| t.contains("drop relation"))),
        ("rename", any(&|t| t.contains("rename relation"))),
        (
            "two at once",
            any(&|t| t.contains("xs") && t.contains("ys")),
        ),
        ("depth 3", migrations.len() == 3),
        ("revert", any(&|t| t.contains("revert"))),
        ("put back", model.slots.iter().flatten().any(|h| h.put_back)),
        (
            "revert at depth 3",
            migrations.len() == 3 && migrations[2].contains("revert"),
        ),
        (
            "put back at depth 3",
            model
                .slots
                .iter()
                .flatten()
                .any(|h| h.put_back && h.added == Some(3)),
        ),
    ]
    .into_iter()
    .filter_map(|(kind, here)| here.then_some(kind))
}

/// Returns the source each log of `cases` leaves, or why it does not replay. A log that is an
/// earlier case's log and one migration more replays that migration over the earlier source.
fn replayed(cases: &[(Vec<String>, Model)]) -> Vec<Result<Source, signature::ReplayError>> {
    let mut done: BTreeMap<&[String], Result<Source, signature::ReplayError>> = BTreeMap::new();
    let mut out = Vec::new();
    for (log, _) in cases {
        let got = match log.split_last() {
            Some((last, before)) if done.contains_key(before) => done[before]
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|s| signature::replay_one(s, log.len() as u64, last)),
            _ => signature::replay(log.iter().map(String::as_str)),
        };
        done.insert(log, got.clone());
        out.push(got);
    }
    out
}

/// Of the histories of three migrations, in the order [`generated`] gives them, the default
/// profile runs one in this many; `deep` runs every history.
const THREE_STRIDE: usize = 8;

#[test]
fn a_pinned_insert_translates_across_every_generated_history_of_two_relations() {
    let pinned = checked(&[PIN]);
    let body = "txn put_x () -> I64 =
  transact
    let r = insert xs { n: 1 }
    return 1
txn put_y () -> I64 =
  transact
    let r = insert ys { n: 1 }
    return 1";
    let transactions = normalised(&pinned, body);
    let every = adtb_testkit::profile::every_case();
    let (mut cases, three): (Vec<_>, Vec<_>) =
        generated(3).into_iter().partition(|(log, _)| log.len() < 4);
    cases.extend(
        three
            .into_iter()
            .enumerate()
            .filter(|(i, _)| every || i % THREE_STRIDE == 0)
            .map(|(_, c)| c),
    );
    cases.push(renamed_then_added());
    let sources = replayed(&cases);
    let mut reached = BTreeSet::new();
    let mut bad = Vec::new();
    for ((log, model), source) in cases.iter().zip(sources) {
        let log: Vec<&str> = log.iter().map(String::as_str).collect();
        let source = match source {
            Ok(source) => source,
            Err(e) => {
                bad.push(format!("{:?}\n  does not replay: {e:?}", &log[1..]));
                continue;
            }
        };
        let compiled = source.check().expect("a replayed version checks");
        let target = (source, compiled);
        let to = target.1.schema();
        let id_of = |name: &str| {
            let rel = to
                .relations
                .iter()
                .position(|r| r.name == name)
                .unwrap_or_else(|| panic!("no relation {name}"));
            to.ids().relation_id(RelId(rel as u32))
        };
        let got = inserted(&pinned, &transactions, &target);
        let want: Vec<Outcome> = (0..2)
            .map(|s| match &model.slots[s] {
                Some(Held {
                    added: None, name, ..
                }) => Ok(id_of(name)),
                _ => Err((
                    model.dropped[s].clone(),
                    format!(
                        "it reaches `{}`, which the current version lacks",
                        SLOTS[s].0
                    ),
                )),
            })
            .collect();
        if want != got {
            bad.push(format!("{:?}\n  want {want:?}\n  got  {got:?}", &log[1..]));
        }
        reached.extend(kinds(&log[1..], model).map(String::from));
        for (s, w) in want.iter().enumerate() {
            reached.insert(format!(
                "{} {}",
                SLOTS[s].0,
                if w.is_ok() { "Ok" } else { "Err" }
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {}:\n{}",
        bad.len(),
        cases.len(),
        bad.join("\n")
    );
    assert_eq!(
        BTreeSet::from(
            [
                "add",
                "depth 3",
                "drop",
                "put back",
                "put back at depth 3",
                "rename",
                "revert",
                "revert at depth 3",
                "two at once",
                "xs Err",
                "xs Ok",
                "ys Err",
                "ys Ok",
            ]
            .map(String::from)
        ),
        reached
    );
}
