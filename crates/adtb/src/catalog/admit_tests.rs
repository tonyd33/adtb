use adtb_lang::schema::ValueDef;
use adtb_lang::types::RelId;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::fixture::Fixture;

use crate::catalog::{Catalog, Error};
use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::storage::Pager;
use crate::test_world::write_path::WritePath;
use crate::test_world::{finished, options};
use crate::write;

/// The value limit the database lowers to, in place of 2^31 - 1.
const VALUE_LIMIT: usize = 6000;

/// The largest version a 4 KiB page holds: the page less its 40-byte header and an 8-byte slot,
/// rounded down to 8.
const ROW_LIMIT: usize = 4048;

/// The length of a version's header.
const HEADER: usize = 24;

/// The length of each of two texts under [`VALUE_LIMIT`] whose concatenation passes it.
const HALF: usize = 3500;

/// The length of a text of [`HALF`] bytes in a sum's payload: a 2-byte length, then its bytes.
const PREFIXED_HALF: usize = 2 + HALF;

/// The number of 16-byte texts a row of a 4 KiB page holds beside its key with less than 18 bytes
/// to spare, so one more such text passes the row limit.
const TEXTS: usize = 221;

/// The longest text that stays inline in a row's smallest form.
const INLINE: usize = 16;

/// The number of texts beside the key of the row a nest grows past the row limit.
const HEADS: usize = 222;

/// The length of the first of those texts, which leaves that row 1 byte under the row limit.
const SHORT_HEAD: usize = 5;

/// The length of the fixed part of a row of an `I64` key and a sum's 1 or 2-byte tag, padded to 2
/// for the offsets that follow it.
const KEY_AND_TAG: usize = 10;

/// Returns the length of the smallest form of a row whose fixed part is `fixed` bytes and whose
/// variable-width fields are `vars` bytes each: the header, the fixed part padded to 2, a 2-byte
/// end offset and a bitmap bit for each variable-width field, then each one's first [`INLINE`]
/// bytes.
fn smallest(fixed: usize, vars: &[usize]) -> usize {
    HEADER
        + fixed.next_multiple_of(2)
        + 2 * vars.len()
        + vars.len().div_ceil(8)
        + vars.iter().map(|&n| n.min(INLINE)).sum::<usize>()
}

fn text(n: usize) -> String {
    format!("\"{}\"", "y".repeat(n))
}

/// Returns what `f` gives for each of `0..n`, joined by commas.
fn joined(n: usize, f: impl Fn(usize) -> String) -> String {
    (0..n).map(f).collect::<Vec<_>>().join(", ")
}

/// What a migration's refusal names: the size of the row or field past a limit.
enum Expected {
    /// A field of this many bytes, past the value limit.
    Value(usize),
    /// A field of the named sum type of this many bytes after its tag, past the value limit.
    Tagged(&'static str, usize),
    /// A row whose smallest form is this many bytes, past the row limit.
    Row(usize),
}

struct Case {
    name: &'static str,
    decls: String,
    /// The statements that write the rows the migration maps.
    rows: Vec<String>,
    migration: String,
    /// The relation and constructor the refusal names.
    target: (&'static str, &'static str),
    expected: Expected,
}

fn cases() -> Vec<Case> {
    let pair = format!("x: {}, y: {}", text(HALF), text(HALF));
    let short = joined(TEXTS, |i| format!("t{i}: {}", text(INLINE)));
    let wide = joined(TEXTS, |i| format!("t{i}: Text"));
    let heads = joined(HEADS, |i| format!("h{i}: Text"));
    let head_values = joined(HEADS, |i| {
        let n = if i == 0 { SHORT_HEAD } else { INLINE };
        format!("h{i}: {}", text(n))
    });
    let mut head_lens = vec![INLINE; HEADS];
    head_lens[0] = SHORT_HEAD;
    assert_eq!(
        ROW_LIMIT - 1,
        smallest(8, &[head_lens.clone(), vec![0]].concat())
    );
    vec![
        Case {
            name: "nest past the value limit",
            decls: "type S = A { x: Text, y: Text } | B { z: I64 }\n\
                    type T { id: I64 } = A { x: Text, y: Text } | B { z: I64 } | C { w: I64 }\n\
                    relation ts : T key id"
                .to_owned(),
            rows: vec![format!("insert ts.A {{ id: 1, {pair} }}")],
            migration: "module app\nnest constructor T.(A | B) into N { s: S }\n".to_owned(),
            target: ("ts", "N"),
            expected: Expected::Tagged("S", 2 * PREFIXED_HALF),
        },
        Case {
            name: "nest past the row limit",
            decls: format!(
                "type S = A {{ x: Text }} | B {{ z: I64 }}\n\
                 type T {{ id: I64, {heads} }} = A {{ x: Text }} | B {{ z: I64 }}\n\
                 relation ts : T key id"
            ),
            rows: vec![format!("insert ts.A {{ id: 1, {head_values}, x: \"\" }}")],
            migration: "module app\nnest constructor T.(A | B) into N { s: S }\n".to_owned(),
            target: ("ts", "N"),
            expected: Expected::Row(smallest(KEY_AND_TAG, &[head_lens, vec![1]].concat())),
        },
        Case {
            name: "merge past the value limit",
            decls: "type P = A | B\n\
                    type T { id: I64 } = A { x: Text, y: Text } | B { x: Text, y: Text } | C { w: I64 }\n\
                    relation ts : T key id"
                .to_owned(),
            rows: vec![format!("insert ts.A {{ id: 1, {pair} }}")],
            migration: "module app\nmerge constructor T.(A | B) into M { p: P, s: Text }\n  by match\n    \
                        | A a -> M { p: A, s: a.x ++ a.y }\n    | B b -> M { p: B, s: b.x ++ b.y }\n"
                .to_owned(),
            target: ("ts", "M"),
            expected: Expected::Value(2 * HALF),
        },
        Case {
            name: "merge past the row limit",
            decls: format!(
                "type P = A | B\n\
                 type T {{ id: I64 }} = A {{ {wide} }} | B {{ {wide} }} | C {{ w: I64 }}\n\
                 relation ts : T key id"
            ),
            rows: vec![format!("insert ts.A {{ id: 1, {short} }}")],
            migration: format!(
                "module app\nmerge constructor T.(A | B) into M {{ p: P, {wide}, e: Text }}\n  by match\n    \
                 | A a -> M {{ p: A, {}, e: a.t0 }}\n    | B b -> M {{ p: B, {}, e: b.t0 }}\n",
                joined(TEXTS, |i| format!("t{i}: a.t{i}")),
                joined(TEXTS, |i| format!("t{i}: b.t{i}")),
            ),
            target: ("ts", "M"),
            expected: Expected::Row(smallest(KEY_AND_TAG, &[INLINE; TEXTS + 1])),
        },
        Case {
            name: "computed fill past the value limit",
            decls: "type T { id: I64 } = A { x: Text, y: Text } | C { w: I64 }\n\
                    relation ts : T key id"
                .to_owned(),
            rows: vec![format!("insert ts.A {{ id: 1, {pair} }}")],
            migration: "module app\nadd field T.d: Text fill match\n    | A a -> a.x ++ a.y\n    \
                        | C c -> \"\"\n"
                .to_owned(),
            target: ("ts", "A"),
            expected: Expected::Value(2 * HALF),
        },
        Case {
            name: "computed fill past the row limit",
            decls: format!(
                "type T {{ id: I64 }} = A {{ {wide} }} | C {{ w: I64 }}\n\
                 relation ts : T key id"
            ),
            rows: vec![format!("insert ts.A {{ id: 1, {short} }}")],
            migration: "module app\nadd field T.d: Text fill match\n    | A a -> a.t0\n    \
                        | C c -> \"\"\n"
                .to_owned(),
            target: ("ts", "A"),
            expected: Expected::Row(smallest(8, &[INLINE; TEXTS + 1])),
        },
    ]
}

/// Run `text` as one statement of `w` and commit it. Returns its failure, the transaction aborted.
fn run(w: &WritePath<SimRuntime>, text: &str) -> Result<(), write::Error> {
    let (terms, item, _) = w.prepare(text, &[]);
    let db = w.db();
    let tx = db.begin().expect("a transaction begins");
    let out = db.statement(&tx, &terms, item, &[], 0).map(drop);
    match out {
        Ok(()) => drop(db.commit(tx).expect("an admitted statement commits")),
        Err(_) => db.abort(tx).expect("an abort"),
    }
    out
}

fn rel(catalog: &Catalog, name: &str) -> RelId {
    let schema = catalog.schema();
    let m = schema.module_named("app").expect("module app");
    match schema.module(m).values.get(name) {
        Some(ValueDef::Relation(r)) => *r,
        _ => panic!("app holds relation {name}"),
    }
}

fn value_past(rel: &str, ctor: &str, len: usize) -> String {
    format!(
        "constructor relation {rel}.{ctor} gets a row with a field stored in {len} bytes, over the value limit of {VALUE_LIMIT}"
    )
}

/// Returns the width of the stored tag of sum type `name` of module app: the fewest of 1, 2 or 4
/// bytes that hold its largest constructor id.
fn tag_width(catalog: &Catalog, name: &str) -> usize {
    let schema = catalog.schema();
    let m = schema.module_named("app").expect("module app");
    let def = schema.module(m).types[name];
    let largest = schema
        .ids()
        .type_ctor_ids(def)
        .map(|(_, id)| u64::from(id))
        .max()
        .expect("a sum has constructors");
    match largest {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        _ => 4,
    }
}

/// Returns the version of `catalog` and the rows of `ts`.
fn state(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> (u64, Vec<(u32, Vec<Value>)>) {
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let rows = catalog
        .rows(pager, txns, &hold, rel(catalog, "ts"))
        .expect("the rows read");
    (catalog.source().version, rows)
}

/// A migration whose nest, merge or computed fill writes a row past the row limit, or a row with
/// a field past the value limit, though each row it reads fits both, is refused naming that
/// relation, constructor, size and limit, each size worked from the row encoding's layout, and
/// commits nothing.
#[test]
fn a_migration_writing_a_row_past_a_limit_is_refused_in_its_report_and_commits_nothing() {
    let mut ran = 0;
    for (seed, case) in cases().into_iter().enumerate() {
        let name = case.name;
        let f = Fixture::new(&case.decls);
        let mut sim = Sim::new(seed as u64, Config::default());
        finished(&mut sim, |rt| {
            let w = WritePath::of_value_limit(&rt, &f, VALUE_LIMIT as u64);
            for row in &case.rows {
                assert_eq!(Ok(()), run(&w, row), "{name}: {row}");
            }
            let pager = w.db().pager().clone();
            drop(w);
            pager.close();
            let (pager, _) = Pager::open(rt.clone(), &options(256)).expect("the file opens");
            pager.steps().lower_value_limit(VALUE_LIMIT as u64);
            let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
            let before = state(&pager, &catalog, &txns);
            let (rel, ctor) = case.target;
            let expected = match case.expected {
                Expected::Value(len) => value_past(rel, ctor, len),
                Expected::Tagged(ty, len) => value_past(rel, ctor, tag_width(&catalog, ty) + len),
                Expected::Row(len) => format!(
                    "constructor relation {rel}.{ctor} gets a row stored in {len} bytes, over the row limit of {ROW_LIMIT}"
                ),
            };
            assert!(!before.1.is_empty(), "{name}: the rows are written");
            assert_eq!(
                Err(Error::Refused(vec![expected])),
                catalog.migrate(&pager, &txns, &case.migration),
                "{name}"
            );
            pager.close();
            let (pager, _) = Pager::open(rt.clone(), &options(256)).expect("the file opens");
            pager.steps().lower_value_limit(VALUE_LIMIT as u64);
            let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
            assert_eq!(
                before,
                state(&pager, &catalog, &txns),
                "{name}: after a reopen"
            );
            pager.close();
        });
        ran += 1;
    }
    assert_eq!(6, ran);
}

/// The length of the stored form of a roots row holding a migration's text of `len` bytes: the
/// sum's 1-byte tag, the version's 8 bytes, then the text's 2-byte length and its bytes.
fn roots_migration_row(len: usize) -> usize {
    1 + 8 + 2 + len
}

/// A migration whose text passes the value limit where the roots heap stores it is refused,
/// naming that row, its size and the limit, and commits nothing.
#[test]
fn a_migration_whose_text_passes_the_value_limit_is_refused_in_its_report() {
    let f = Fixture::new("type U = { id: I64 }\nrelation us : U key id");
    let mut sim = Sim::new(7, Config::default());
    finished(&mut sim, |rt| {
        let w = WritePath::of_value_limit(&rt, &f, VALUE_LIMIT as u64);
        let pager = w.db().pager().clone();
        drop(w);
        pager.close();
        let (pager, _) = Pager::open(rt.clone(), &options(256)).expect("the file opens");
        pager.steps().lower_value_limit(VALUE_LIMIT as u64);
        let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
        let version = catalog.source().version;
        let text = format!(
            "module app\n-- {}\nadd type W = W1 | W2\n",
            "y".repeat(VALUE_LIMIT)
        );
        let expected = format!(
            "the roots row holding the text of migration {} is stored in {} bytes, over the value limit of {VALUE_LIMIT}",
            version + 1,
            roots_migration_row(text.len())
        );
        assert_eq!(
            Err(Error::Refused(vec![expected])),
            catalog.migrate(&pager, &txns, &text)
        );
        pager.close();
        let (pager, _) = Pager::open(rt.clone(), &options(256)).expect("the file opens");
        let (catalog, _) = Catalog::open(rt, &pager).expect("the catalog opens");
        assert_eq!(version, catalog.source().version);
        pager.close();
    });
}
