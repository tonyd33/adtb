use super::*;
use crate::parse::{Options, parse_bundle_body};
use crate::signature::{self, Item};

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

fn checked(log: &[&str]) -> (Source, Schema, Program) {
    let source = signature::replay(log.iter().copied()).expect("the log replays");
    let (schema, program) = source.check().expect("the version checks");
    (source, schema, program)
}

/// Returns the translation of the one query or transaction of `body`, checked at the version
/// the first `pin` migrations of `log` leave, to the version `log` leaves.
fn translated(log: &[&str], pin: usize, body: &str) -> R<NormalQuery> {
    let (from_source, from, from_program) = checked(&log[..pin]);
    let (to_source, to, to_program) = checked(log);
    let text = format!("import auth\n{body}");
    let (imports, items) = parse_bundle_body(&text, Options::default())
        .ok()
        .expect("the body parses");
    let vis = signature::visible(&from, &imports).expect("auth is visible");
    let calls = signature::calls(&from, &from_program, &vis, &imports, &items)
        .unwrap_or_else(|d| panic!("`{body}` checks at its pin: {}", d.message));
    let q = match &calls[0].item {
        Item::Query(_, q) => norm::query(&from, &from_program, q),
        Item::Txn(_, t) => norm::txn(&from, &from_program, t),
        Item::Fn(..) => panic!("the body holds a query or transaction"),
    }
    .expect("the callable normalises");
    Pinned::new(&from, &from_source, &to, &to_source, &to_program, false)?.query(&q)
}

/// Returns the operator and reason of the refusal of translating `body`.
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
fn a_rename_maps_every_object_by_its_catalog_id() {
    let (_, from, _) = checked(&[V1]);
    let (_, to, _) = checked(&[V1, RENAMES]);
    let map = Map::new(&from, &to);
    let tier = named(&from, "Tier");
    let plan = named(&to, "Plan");
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
    let user = named(&from, "User");
    assert_eq!(
        Some("display".to_string()),
        map.field(user, Some(0), "name")
    );
}

#[test]
fn a_removed_constructor_has_no_image_and_an_added_one_no_preimage() {
    let (_, from, _) = checked(&[V1, ADD_GOLD]);
    let (_, to, _) = checked(&[V1, ADD_GOLD, DEPRECATE_FREE, REMOVE_FREE]);
    let map = Map::new(&from, &to);
    let tier = named(&from, "Tier");
    assert_eq!(None, map.ctor(tier, 0));
    assert_eq!(Some(0), map.ctor(tier, 1));
    assert_eq!(Some(1), map.ctor_back(tier, 0));
    let (_, pin, _) = checked(&[V1]);
    let (_, gold, _) = checked(&[V1, ADD_GOLD]);
    let added = Map::new(&pin, &gold);
    assert_eq!(None, added.ctor_back(tier, 2));
}

#[test]
fn hide_new_narrows_references_and_rows_to_the_pinned_constructors() {
    const ADD_APPLE: &str = "module auth\nadd constructor Identity.Apple { sub: Text }\n";
    let (_, from, _) = checked(&[V1]);
    let (_, to, _) = checked(&[V1, ADD_APPLE]);
    let map = Map::new(&from, &to);
    let identities = RelId(
        from.relations
            .iter()
            .position(|r| r.name == "identities")
            .expect("identities") as u32,
    );
    let identity = named(&from, "Identity");
    let key = Ty::Key {
        rel: identities,
        ctors: None,
    };
    let row = Ty::Data {
        def: identity,
        args: Vec::new(),
        ctors: None,
    };
    assert_eq!(Ok(key.clone()), map.ty(&key, false));
    let pinned = Some(vec![0, 1]);
    assert_eq!(
        Ok(Ty::Key {
            rel: identities,
            ctors: pinned.clone()
        }),
        map.ty(&key, true)
    );
    assert_eq!(
        Ok(Ty::Data {
            def: identity,
            args: Vec::new(),
            ctors: pinned
        }),
        map.ty(&row, true)
    );
}

#[test]
fn a_dropped_field_has_no_image() {
    let (_, from, _) = checked(&[V1]);
    let (_, to, _) = checked(&[V1, DROP_GID]);
    let map = Map::new(&from, &to);
    let identity = named(&from, "Identity");
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
    assert!(reason.contains("gid"), "{reason}");
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
    assert!(reason.contains("counts"), "{reason}");
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
    assert!(reason.contains("seats"), "{reason}");
}

#[test]
fn a_parameter_or_comparison_reaching_a_removed_constructor_is_refused() {
    let log = [V1, ADD_GOLD, DEPRECATE_FREE, REMOVE_FREE];
    for body in [
        "query q (t: Tier) -> Set Text = from u in users select u.name",
        "query q () -> Set Text = from u in users where u.tier == Gold select u.name",
    ] {
        let (op, reason) = refused_across(&log, 2, body);
        assert_eq!("remove constructor Tier.Free", op, "{body}");
        assert!(reason.contains("Tier.Free"), "{reason}");
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
    assert!(reason.contains("Identity.Google"), "{reason}");
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
    assert!(reason.contains("outside a write"), "{reason}");
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
    let test = terms.add(Kind::Var(c), Ty::Unit);
    let row = terms.var("x", Ty::Unit);
    let element = terms.add(Kind::Var(row), Ty::Unit);
    let yielded = terms.add(Kind::Singleton(element), Ty::Unit);
    let set = terms.doubled(test, yielded, 64);
    assert!(walk::yields(&terms, set, row));
}
