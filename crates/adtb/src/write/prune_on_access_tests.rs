//! A writer finding a page without room prunes it before taking another page.

use adtb_harness::process;

use crate::OpenOptions;
use crate::test_world::api;
use crate::test_world::load::{self as open, options};
use crate::testing::Knobs;

const APP: &str = "type Thing { id: Key things, note: Text } = A { a: Text } | B { b: Text }
relation things : Thing key id generate sequence
";

fn note() -> String {
    "n".repeat(200)
}

#[test]
fn a_retag_into_a_heap_whose_only_page_is_full_of_dead_versions_reuses_that_page() {
    process::once(1, |rt| {
        let opts = OpenOptions {
            testing: Knobs::without_background_vacuum(),
            ..options(1)
        };
        let db = open::open_with(&rt, APP, &opts);
        let t = db.testing();
        let (a, b) = (
            t.heap("app::things", "A").unwrap(),
            t.heap("app::things", "B").unwrap(),
        );
        let mut k = 0;
        while t.pages(a).unwrap().len() < 2 {
            k += 1;
            api::results(
                &db,
                &format!(
                    r#"insert things.A {{ id: things.key({k}), note: "{}", a: "x" }}"#,
                    note()
                ),
            );
        }
        let fits = k - 1;
        let first = k + 1;
        for j in first..first + fits {
            api::results(
                &db,
                &format!(
                    r#"insert things.B {{ id: things.key({j}), note: "{}", b: "x" }}"#,
                    note()
                ),
            );
        }
        let pages = t.pages(b).unwrap();
        assert_eq!(1, pages.len(), "the versions of B sit on one page");
        api::results(&db, "delete t in things.B");
        let allocated = t.allocated().unwrap();
        api::results(
            &db,
            r#"update t in things.A where key t == things.key(1) set B { t with b: "x" }"#,
        );
        assert_eq!(pages, t.pages(b).unwrap());
        assert_eq!(allocated, t.allocated().unwrap());
        db.close().unwrap();
    });
}
