//! A page of a heap's directory relabelled to another kind, or holding a page of another owner,
//! is refused by every write that meets it as a read refuses it, and is left unchanged.

use adtb::testing::{PageKind, Pid, page, page::FIRST_EDITABLE};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb::batch::BatchBuilder;
use adtb::key::Value;
use adtb::{ArrowSchema, CallOptions, Database, Error, FatalReason, LoadOptions, Log, RecordBatch};
use adtb_harness::damage::{DataFile, Files};
use adtb_harness::process;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::open;

type Write = fn(&Database<SimRuntime>, Target) -> Result<(), Error>;

const SEED: u64 = 73;
const FILE: &str = "heap_kind.adt";
const PAGE: usize = 8192;
const DATA: DataFile = DataFile::new(FILE, PAGE);
const KIND: usize = 14;
const APP: &str = r"module app
add module app
add type Identity { id: Key identities, tag: I64 }
  = Google { gid: I64 }
  | Apple  { sub: Text }
add relation identities : Identity key id generate sequence
";

/// A constructor whose heap holds the page replaced: its own field, the key of its first row, the
/// kind of its pages and the other heap kind, and what a retag sets its row to.
#[derive(Clone, Copy, Debug)]
struct Target {
    ctor: &'static str,
    field: (&'static str, &'static str),
    key: u32,
    kinds: (PageKind, PageKind),
    retag: &'static str,
}

#[rustfmt::skip]
const TARGETS: [Target; 2] = [
    Target { ctor: "Google", field: ("gid", "7"), key: 1, kinds: (PageKind::HeapFixed, PageKind::HeapSlotted), retag: r#"Apple { i with sub: "r" }"# },
    Target { ctor: "Apple", field: ("sub", r#""s""#), key: 11, kinds: (PageKind::HeapSlotted, PageKind::HeapFixed), retag: "Google { i with gid: 3 }" },
];

impl Target {
    fn row(self, k: u32) -> String {
        let (c, (f, v)) = (self.ctor, self.field);
        format!("identities.{c} {{ id: identities.key({k}), tag: {k}, {f}: {v} }}")
    }
}

#[derive(Clone, Copy, Debug)]
enum Replacement {
    Kind(PageKind),
    /// The first page of the large-value store, a heap page of another owner.
    Large,
}

/// The fixture's files, the first page of each target's heap and of the large-value store, and
/// the versions of each target's heap.
struct Fixture {
    files: Files,
    pages: BTreeMap<&'static str, Pid>,
    versions: BTreeMap<&'static str, String>,
}

/// Returns the TID, header and flags of every version of `t`'s heap.
fn headers(db: &Database<SimRuntime>, t: Target) -> String {
    let testing = db.testing();
    let heap = testing.heap("app::identities", t.ctor).unwrap();
    format!("{:?}", testing.headers(heap).unwrap())
}

impl Fixture {
    fn new() -> Fixture {
        let mut sim = Sim::new(SEED, Config::default());
        let long = format!(
            r#"identities.Apple {{ id: identities.key(20), tag: 0, sub: "{}" }}"#,
            "l".repeat(3 * PAGE)
        );
        let (pages, versions) = process::run(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            for t in TARGETS {
                api::results(
                    &db,
                    &format!("insert {}\ninsert {}", t.row(t.key), t.row(t.key + 1)),
                );
            }
            api::results(&db, &format!("insert {long}"));
            let testing = db.testing();
            let (mut pages, mut versions) = (BTreeMap::new(), BTreeMap::new());
            for t in TARGETS {
                versions.insert(t.ctor, headers(&db, t));
                let heap = testing.heap("app::identities", t.ctor).unwrap();
                let pid = testing.pages(heap).unwrap()[0];
                assert_eq!(
                    t.kinds.0 as u16,
                    page::kind(&testing.probe().page(pid).unwrap()),
                    "{t:?}"
                );
                pages.insert(t.ctor, pid);
                let large = testing.large("app::identities", t.ctor).unwrap();
                pages.insert("large", testing.pages(large).unwrap()[0]);
            }
            db.close().unwrap();
            (pages, versions)
        });
        Fixture {
            files: sim.files(),
            pages,
            versions,
        }
    }

    /// Run `write` on the fixture with the first page of `t`'s heap replaced as `r` says, and
    /// assert it refuses as the heap quarantined, damaging only that page or, when `root`, only the
    /// directory's root, and leaves the page and the heap's versions unchanged.
    fn refuses(&self, t: Target, r: Replacement, form: &str, write: Write, root: bool) {
        let what = format!("{t:?} replaced by {r:?}, {form}");
        let pid = self.pages[t.ctor];
        let large = DATA.page(&self.files, self.pages["large"]).to_vec();
        let mut files = self.files.clone();
        let page = DATA.page_mut(&mut files, pid);
        match r {
            Replacement::Kind(k) => page[KIND..KIND + 2].copy_from_slice(&(k as u16).to_le_bytes()),
            Replacement::Large => page[FIRST_EDITABLE..].copy_from_slice(&large[FIRST_EDITABLE..]),
        }
        page::seal(pid, page);
        let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
        process::run(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            let got = write(&db, t);
            let Err(Error::Fatal {
                reason: FatalReason::Quarantined { what: holder },
            }) = got
            else {
                panic!("{what}: not Quarantined: {got:?}");
            };
            assert_eq!(format!("app::identities.{}", t.ctor), holder, "{what}");
            let testing = db.testing();
            let heap = testing.heap("app::identities", t.ctor).unwrap();
            let met: BTreeSet<Pid> = testing.probe().refusals().into_keys().collect();
            let blamed = BTreeSet::from([if root {
                testing.root(heap).unwrap()
            } else {
                pid
            }]);
            assert_eq!(blamed, met, "{what}");
            let only = BTreeSet::from([testing.own_mark(heap).unwrap()]);
            assert_eq!(only, db.testing().probe().marks(), "{what}");
            db.close().unwrap();
        });
        let mut after = sim.files();
        assert_eq!(
            DATA.page(&files, pid),
            DATA.page(&after, pid),
            "{what}: the page changed"
        );
        DATA.page_mut(&mut after, pid)
            .copy_from_slice(DATA.page(&self.files, pid));
        let mut sim = Sim::with_files(SEED, Config::default(), after);
        let versions = process::run(&mut sim, |rt| {
            headers(
                &open::opened(&rt, FILE, &Log::new([APP]), &open::options(2)),
                t,
            )
        });
        assert_eq!(
            self.versions[t.ctor], versions,
            "{what}: the versions changed"
        );
    }
}

fn run(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

fn insert(db: &Database<SimRuntime>, t: Target) -> Result<(), Error> {
    run(db, &format!("insert {}", t.row(9)))
}

fn load(db: &Database<SimRuntime>, t: Target) -> Result<(), Error> {
    let mut load = db.load(&LoadOptions::default())?;
    let mut a = load.appender(format!("identities.{}", t.ctor).as_str())?;
    let fields = a.schema().fields.iter().filter(|f| f.name != "id");
    let schema = Arc::new(ArrowSchema {
        fields: fields.cloned().collect(),
    });
    let own = match t.kinds.0 {
        PageKind::HeapFixed => Value::I64(7),
        _ => Value::Text("s".into()),
    };
    let mut b = BatchBuilder::new(schema.batch_schema());
    b.push_row(&[Value::I64(9), own]);
    a.push(&RecordBatch::new(schema, b.finish()).unwrap())?;
    load.commit().map(drop)
}

#[rustfmt::skip]
const FORMS: [(&str, Write); 7] = [
    ("insert", insert),
    ("update", |db, t| run(db, &format!("update i in identities.{0} where key i == identities.key({1}) set {0} {{ i with tag: 5 }}", t.ctor, t.key))),
    ("upsert", |db, t| run(db, &format!("insert {} unless conflict on key else match old | Google o -> update o set Google {{ o with tag: 5 }} | Apple o -> update o set Apple {{ o with tag: 5 }}", t.row(t.key)))),
    ("delete", |db, t| run(db, &format!("delete i in identities where key i == identities.key({})", t.key))),
    ("retag", |db, t| run(db, &format!("update i in identities.{} where key i == identities.key({}) set {}", t.ctor, t.key, t.retag))),
    ("vacuum", |db, _| run(db, "vacuum identities")),
    ("load", load),
];

const SCAN: Write = |db, t| run(db, &format!("from i in identities.{} select i.tag", t.ctor));

#[rustfmt::skip]
const READS: [(&str, Write); 2] = [
    ("vacuum", |db, _| run(db, "vacuum identities")),
    ("index read", |db, t| run(db, &format!("from i in identities.{} where i.id == identities.key({}) select i.tag", t.ctor, t.key))),
];

#[test]
fn an_insert_refuses_a_heap_page_of_another_kind_or_owner_as_a_read_does() {
    let ours = |t: Target, r| matches!(r, Replacement::Kind(k) if k == t.kinds.1);
    let fixture = Fixture::new();
    for t in TARGETS {
        let maps = [PageKind::FreeSpaceMap, PageKind::VisibilityMap].map(Replacement::Kind);
        for r in [Replacement::Kind(t.kinds.1), Replacement::Large]
            .into_iter()
            .chain(maps)
        {
            fixture.refuses(t, r, "insert", insert, !ours(t, r));
            fixture.refuses(t, r, "scan", SCAN, !ours(t, r));
        }
    }
}

#[test]
fn a_vacuum_and_an_index_read_blame_the_page_a_scan_blames() {
    let ours = |t: Target, r| matches!(r, Replacement::Kind(k) if k == t.kinds.1);
    let fixture = Fixture::new();
    for t in TARGETS {
        let maps = [PageKind::FreeSpaceMap, PageKind::VisibilityMap].map(Replacement::Kind);
        for r in [Replacement::Kind(t.kinds.1), Replacement::Large]
            .into_iter()
            .chain(maps)
        {
            for (form, read) in READS {
                fixture.refuses(t, r, form, read, !ours(t, r));
            }
        }
    }
}

#[test]
fn every_write_refuses_a_heap_page_relabelled_to_the_other_heap_kind() {
    let fixture = Fixture::new();
    for t in TARGETS {
        for (form, write) in FORMS {
            fixture.refuses(t, Replacement::Kind(t.kinds.1), form, write, false);
        }
    }
}
