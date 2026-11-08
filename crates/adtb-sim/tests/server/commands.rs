//! Commands of ad hoc text run in process: explain, explain analyze, vacuum and set setting.

use adtb_simkit::api;

use adtb::{Database, Log};
use adtb_sim::SimRuntime;

use adtb_simkit::open::options;
use api::{once, printed, result, results};

const SCHEMA: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  unique (user) in Google
  index (user) in Google
  index (user)
";

fn with_db<T>(body: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    once(0, |rt| {
        let db = Database::open_in(rt, "commands.adt", &Log::new([SCHEMA]), &options(2))
            .expect("the database opens");
        results(
            &db,
            r#"insert users { id: users.key(1), name: "ann" }
               insert users { id: users.key(2), name: "bo" }
               insert identities.EmailPass { id: identities.key(1), user: users.key(1), email: "a@x" }
               insert identities.Google { id: identities.key(2), user: users.key(2), gid: "g-bo" }"#,
        );
        body(&db)
    })
}

#[test]
fn explain_results_take_their_place_among_the_items() {
    with_db(|db| {
        let out = results(
            db,
            r"(count(u for u in users))
              explain from u in users select u.name
              (count(i for i in identities))",
        );
        assert_eq!(3, out.len());
        let shown: Vec<Vec<String>> = out.into_iter().map(printed).collect();
        assert_eq!(vec!["2".to_owned()], shown[0]);
        assert!(shown[1][0].contains("Scan users.User"), "{:?}", shown[1]);
        assert_eq!(vec!["2".to_owned()], shown[2]);
    });
}

#[test]
fn vacuum_returns_one_row_per_constructor_relation() {
    with_db(|db| {
        results(db, "delete i in identities where value(key i) == 1");
        let got = printed(results(db, "vacuum identities").remove(0));
        assert_eq!(3, got.len(), "{got:?}");
        assert!(
            got.iter()
                .all(|r| r.contains("ctor: ") && r.contains("removed: ")),
            "{got:?}"
        );
        assert_eq!(
            vec!["2".to_owned()],
            printed(result(db, "from i in identities select value(key i)")),
            "vacuum removes no live row"
        );
    });
}

#[test]
fn vacuum_removes_versions_no_snapshot_sees() {
    with_db(|db| {
        results(db, "delete i in identities where value(key i) == 1");
        let got = printed(results(db, "vacuum identities").remove(0));
        assert!(
            got.iter().any(|r| !r.contains("removed: 0")),
            "the deleted EmailPass version is removed: {got:?}"
        );
    });
}

#[test]
fn set_setting_replaces_the_row_of_its_constructor() {
    with_db(|db| {
        let got = printed(results(db, "set setting RetryLimit = { retries: 3 }").remove(0));
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("RetryLimit {"), "{got:?}");
        assert!(got[0].contains("retries: 3"), "{got:?}");
        results(db, "set setting RetryLimit = { retries: 7 }");
        assert_eq!(
            vec!["7".to_owned()],
            printed(result(
                db,
                "from s in catalog::settings select match s
                   | RetryLimit r -> r.retries
                   | _ -> 0"
            ))
            .into_iter()
            .filter(|r| r != "0")
            .collect::<Vec<_>>()
        );
    });
}
