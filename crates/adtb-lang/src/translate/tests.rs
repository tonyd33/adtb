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
    pinned: &(Source, Schema, Program),
    log: &[&str],
    body: &str,
    hide_new: bool,
) -> Vec<R<NormalQuery>> {
    let (from_source, from, from_program) = pinned;
    let (to_source, to, to_program) = checked(log);
    let text = format!("import auth\n{body}");
    let (imports, items) = parse_bundle_body(&text, Options::default())
        .ok()
        .expect("the body parses");
    let vis = signature::visible(from, &imports).expect("auth is visible");
    let calls = signature::calls(from, from_program, &vis, &imports, &items)
        .unwrap_or_else(|d| panic!("`{body}` checks at its pin: {}", d.message));
    let p = Pinned::new(from, from_source, &to, &to_source, &to_program, hide_new);
    calls
        .iter()
        .map(|c| {
            let q = match &c.item {
                Item::Query(_, q) => norm::query(from, from_program, q),
                Item::Txn(_, t) => norm::txn(from, from_program, t),
                Item::Fn(..) => panic!("the body holds a query or transaction"),
            }
            .expect("the callable normalises");
            p.as_ref().map_err(Clone::clone)?.query(&q)
        })
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
        let (_, current, _) = checked(&log);
        for (hide_new, expected) in [(false, probe.clone()), (true, vec![0])] {
            let q = translated_with_hiding(&log, 2, body, hide_new)
                .unwrap_or_else(|e| panic!("{name}: the upsert translates: {e:?}"));
            let scopes: Vec<_> = q
                .terms
                .nodes
                .iter()
                .filter_map(|n| match n.kind {
                    Kind::Upsert {
                        rel,
                        on: crate::kernel::Conflict::Unique(i),
                        old,
                        ..
                    } => {
                        let Ty::Data { ctors, .. } = &q.terms.var_info(old).ty else {
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
        rel: RelRef::Declared(identities),
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
            rel: RelRef::Declared(identities),
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
    assert!(forms::yields(&terms, set, row));
}

#[test]
fn a_wasm_function_taken_to_another_module_conflicts_with_a_pin_of_the_old_one() {
    let wasm = |m: &str, op: &str| {
        format!("module auth\n{op} fn f (t: Text) -> Text = wasm \"{m}\" export \"e\"\n")
    };
    let (define, redefine) = (wasm("m", "define"), wasm("n", "redefine"));
    let (at_pin, _, _) = checked(&[V1, &define]);
    let (next, _, _) = checked(&[V1, &define, &redefine]);
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
    let (at_pin, _, _) = checked(&[V1, host]);
    let (before, _, _) = checked(&[V1, host, reader]);
    let (next, _, _) = checked(&[V1, host, reader, rename]);
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
    let (_, from, _) = checked(&[V1, add]);
    let (_, to, _) = checked(&[V1, add, down]);
    let map = Map::new(&from, &to);
    let def = named(&from, "Identity");
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
