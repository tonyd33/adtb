//! A page the file holds at an LSN past the log's end, met by every entry that reads or writes it.

use std::collections::BTreeMap;
use std::time::Duration;

use adtb::runtime::Runtime;
use adtb::storage::{self, Pid, Quarantine};
use adtb::{
    ArchiveTarget, CallOptions, Database, Error, FatalReason, Log, OpenOptions, RestorePoint,
    restore_in,
};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::{api, common};

type Files = BTreeMap<String, Vec<u8>>;

const SEED: u64 = 61;
const FILE: &str = "past_end.adt";
const PAGE: usize = 8192;
const PAST: u64 = 1 << 40;
const GOOGLE: &str = "app::identities.Google";
const APP: &str = r"module app
add module app
add type Identity { id: Key identities, tag: Text }
  = Google { gid: Text }
  | Apple  { sub: Text }
add relation identities : Identity key id generate sequence
  index (tag)
";

/// What an entry answered.
#[derive(Debug, PartialEq)]
enum Met {
    Refused(String),
    Found(Vec<String>),
    Copied(Vec<u64>),
    Finished,
    Other(String),
}

type Enter = fn(&Database<SimRuntime>, &SimRuntime) -> Met;

fn open(rt: SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        no_background_vacuum: true,
        ..api::options(2)
    };
    Database::open_in(rt, FILE, &Log::new([APP]), &options).expect("the file opens")
}

/// Returns a closed file whose one page of `identities.Google` carries LSN `PAST`, and that page.
fn planted() -> (Files, Pid) {
    let mut sim = Sim::new(SEED, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open(rt);
        for i in 1..=3 {
            api::results(
                &db,
                &format!(
                    r#"insert identities.Google {{ id: identities.key({i}), tag: "t{i}", gid: "g{i}" }}
                       insert identities.Apple {{ id: identities.key({}), tag: "u{i}", sub: "s{i}" }}"#,
                    i + 10
                ),
            );
        }
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Google").unwrap();
        let pages = heap.pages(w.pager()).unwrap();
        assert_eq!(1, pages.len(), "one page holds every Google row");
        db.close().unwrap();
        pages[0]
    });
    let mut files = sim.files();
    let at = page.0 as usize * PAGE;
    let bytes = &mut files.get_mut(FILE).unwrap()[at..at + PAGE];
    bytes[..8].copy_from_slice(&PAST.to_le_bytes());
    storage::seal(page, bytes);
    (files, page)
}

fn answered(got: Result<Vec<adtb::DynRows>, Error>) -> Met {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => Met::Refused(what),
        Ok(_) => Met::Other("ran".to_owned()),
        Err(e) => Met::Other(format!("{e:?}")),
    }
}

fn text(db: &Database<SimRuntime>, text: &str) -> Met {
    answered(db.query_text(text, &[], &CallOptions::default()))
}

fn verify(db: &Database<SimRuntime>, relation: Option<&str>) -> Met {
    let found = db.verify(relation).map(api::printed);
    found.map_or_else(|e| Met::Other(format!("{e:?}")), Met::Found)
}

fn scrub(db: &Database<SimRuntime>, rt: &SimRuntime) -> Met {
    let every = "Some { value: 1h }";
    api::results(
        db,
        &format!("set setting ScrubInterval = {{ every: {every} }}"),
    );
    common::sleep(rt, &mut rt.io(), Duration::from_secs(3600 + 60));
    let found = api::result(db, "from f in sys::verify_findings select f");
    Met::Found(api::sorted(found))
}

fn job(db: &Database<SimRuntime>, rt: &SimRuntime, start: &str) -> Met {
    if let Err(e) = db.query_text(start, &[], &CallOptions::default()) {
        return answered(Err(e));
    }
    for _ in 0..600 {
        if api::printed(api::result(db, "from j in sys::jobs select j.id")).is_empty() {
            return Met::Finished;
        }
        common::sleep(rt, &mut rt.io(), Duration::from_millis(100));
    }
    let jobs = api::printed(api::result(db, "from j in sys::jobs select j"));
    Met::Other(format!("the job never finished: {jobs:?}"))
}

fn backup(db: &Database<SimRuntime>, rt: &SimRuntime) -> Met {
    api::results(db, "backup to \"bk\"");
    let archive = ArchiveTarget::path("bk").unwrap();
    let options = OpenOptions {
        create: None,
        ..api::options(2)
    };
    let latest = RestorePoint::Latest;
    let restored = restore_in(rt.clone(), &archive, "copy.adt", latest, &options);
    restored.map_or_else(|e| Met::Other(format!("{e:?}")), Met::Copied)
}

#[rustfmt::skip]
const ENTRIES: [(&str, Enter); 12] = [
    ("query", |db, _| text(db, "from i in identities.Google select i.gid")),
    ("insert", |db, _| text(db, r#"insert identities.Google { id: identities.key(9), tag: "t9", gid: "g9" }"#)),
    ("update", |db, _| text(db, r#"update i in identities.Google where key i == identities.key(1) set Google { i with gid: "h" }"#)),
    ("delete", |db, _| text(db, "delete i in identities where key i == identities.key(1)")),
    ("retag", |db, _| text(db, r#"update i in identities.Google where key i == identities.key(1) set Apple { i with sub: "r" }"#)),
    ("vacuum", |db, _| text(db, "vacuum identities")),
    ("reindex", |db, rt| job(db, rt, "reindex identities")),
    ("dump", |db, _| text(db, "dump to \"d\"")),
    ("verify", |db, _| verify(db, None)),
    ("verify identities", |db, _| verify(db, Some("identities"))),
    ("scrub", scrub),
    ("backup", backup),
];

/// Returns why `met` is not what the spec states for `entry` meeting page `page`.
fn wrong(entry: &str, page: Pid, met: &Met) -> Option<String> {
    let stale = format!("page {}: Stale {{", page.0);
    let unlogged = format!("Unlogged {{ pid: Pid({}), page: Lsn({PAST}) }}", page.0);
    let right = match met {
        Met::Refused(what) => {
            !matches!(entry, "verify" | "verify identities" | "scrub" | "backup") && what == GOOGLE
        }
        Met::Found(rows) => {
            matches!(entry, "verify" | "verify identities" | "scrub")
                && rows.len() == 2
                && rows
                    .iter()
                    .any(|r| r.starts_with("StalePage") && r.contains(&stale))
                && rows
                    .iter()
                    .any(|r| r.starts_with("Structure") && r.contains(&unlogged))
        }
        Met::Copied(damaged) => entry == "backup" && *damaged == [page.0],
        Met::Finished => entry == "reindex",
        Met::Other(_) => false,
    };
    (!right).then(|| format!("{entry}: {met:?}"))
}

#[test]
fn every_entry_refuses_or_reports_a_page_past_the_log_end_and_leaves_it_unchanged() {
    let (files, page) = planted();
    let at = page.0 as usize * PAGE;
    let bytes = files[FILE][at..at + PAGE].to_vec();
    let mut defects = Vec::new();
    for (entry, enter) in ENTRIES {
        let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
        let marked = api::run(&mut sim, |rt| {
            let db = open(rt.clone());
            defects.extend(wrong(entry, page, &enter(&db, &rt)));
            let w = db.write_path();
            let heap = w.heap_named("app::identities", "Google").unwrap().id().0;
            let marked = w.pager().is_quarantined(Quarantine::Heap(heap));
            if let Err(e) = db.close()
                && !matches!(
                    e,
                    Error::Fatal {
                        reason: FatalReason::Quarantined { .. }
                    }
                )
            {
                defects.push(format!("{entry}: close: {e:?}"));
            }
            marked
        });
        if !marked && entry != "backup" {
            defects.push(format!("{entry}: {GOOGLE} is not quarantined"));
        }
        if sim.files()[FILE][at..at + PAGE] != bytes[..] {
            defects.push(format!("{entry}: the page changed"));
        }
    }
    assert_eq!(Vec::<String>::new(), defects);
}
