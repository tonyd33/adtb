use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;

use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::storage::{Damage, Opening, Options, Pager, Quarantine};
use crate::testing::Probe;

use super::{
    CreateOptions, Database, Log, OpenError, OpenOptions, file, mark_failed_open_damage,
    open_damage,
};

fn unreadable_root(scratch: &Scratch) -> (OsRuntime, Pager<OsRuntime>, Opening) {
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = Options::new("unreadable.adt");
    let pager = Pager::create(rt.clone(), &options).unwrap();
    drop(Catalog::open(rt.clone(), &pager).unwrap());
    pager.close();
    let (pager, opening) = Pager::open(rt.clone(), &options).unwrap();
    pager.plant_damage(pager.catalog_root(), Damage::Unreadable);
    (rt, pager, opening)
}

#[test]
fn an_unreadable_catalog_root_marks_nothing_and_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-unmarked");
    let (rt, pager, opening) = unreadable_root(&scratch);
    mark_failed_open_damage(rt, &pager, &opening);
    assert!(!pager.is_quarantined(Quarantine::Database));
    assert!(open_damage(&pager).is_none());
    pager.close();
}

#[test]
fn an_unreadable_catalog_root_leaves_a_damaged_page_unmapped() {
    let scratch = Scratch::new("open-damage", "unreadable-unmapped");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = Options::new("unreadable.adt");
    let pager = Pager::create(rt.clone(), &options).unwrap();
    let (_, txns) = Catalog::open(rt.clone(), &pager).unwrap();
    let damaged = txns.commit_log().root();
    drop(txns);
    pager.close();
    let (pager, opening) = Pager::open(rt.clone(), &options).unwrap();
    pager.plant_damage(damaged, Damage::Checksum);
    pager.plant_damage(pager.catalog_root(), Damage::Unreadable);
    mark_failed_open_damage(rt, &pager, &opening);
    assert_eq!(
        (None, BTreeSet::new()),
        (pager.mapped_to(damaged), Probe::of(&pager).marks())
    );
    pager.close();
}

#[test]
fn an_unreadable_page_on_a_marked_file_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-marked");
    let (_rt, pager, opening) = unreadable_root(&scratch);
    pager.plant_mark(Quarantine::Database, &opening).unwrap();
    assert!(open_damage(&pager).is_none());
    pager.close();
}

const NOTES: &str = r#"module shop
add module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
"#;

const SLUGGED: &str = r"module shop
add constraint slugged =
  from n in notes
  where slugify(n.body) != n.body
  select n
";

#[derive(Clone, Copy, Debug)]
enum Creation {
    Missing,
    Existing,
    Replaced,
    Unlinked,
}

#[test]
fn a_refused_open_keeps_only_files_it_found_across_creation_races() {
    let mut ran = 0;
    for (case, retained, collisions) in [
        (Creation::Missing, false, 0),
        (Creation::Existing, true, 1),
        (Creation::Replaced, true, 1),
        (Creation::Unlinked, false, 1),
    ] {
        let scratch = Scratch::new("open-provenance", &format!("{case:?}"));
        let path = scratch.path().join("notes.adt");
        let log_dir = scratch.path().join("notes.adt-log");
        let sentinel = scratch.path().join("unrelated");
        std::fs::write(&sentinel, b"keep me").unwrap();
        let inode = Rc::new(Cell::new(0));
        if !matches!(case, Creation::Missing) {
            let original = File::create(&path).unwrap();
            inode.set(original.metadata().unwrap().ino());
        }
        let hits = Rc::new(Cell::new(0));
        let action_hits = Rc::clone(&hits);
        let action_inode = Rc::clone(&inode);
        let action_path = path.clone();
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(1),
            ..OpenOptions::default()
        };
        let got = file::with_collision(
            &path,
            move || {
                action_hits.set(action_hits.get() + 1);
                let old = File::options()
                    .read(true)
                    .write(true)
                    .open(&action_path)
                    .unwrap();
                old.try_lock().unwrap();
                match case {
                    Creation::Missing => panic!("an absent file has no create collision"),
                    Creation::Existing => {}
                    Creation::Unlinked => std::fs::remove_file(&action_path).unwrap(),
                    Creation::Replaced => {
                        let aside = action_path.with_file_name("replacement");
                        let replacement = File::create(&aside).unwrap();
                        action_inode.set(replacement.metadata().unwrap().ino());
                        std::fs::rename(aside, &action_path).unwrap();
                    }
                }
            },
            || Database::open(&path, &Log::new([NOTES, SLUGGED]), &options).map(drop),
        );
        let Err(OpenError::HostFnMissing { name }) = got else {
            panic!("{case:?}: expected the missing host refusal, found {got:?}");
        };
        assert_eq!("slugify", name, "{case:?}");
        assert_eq!(collisions, hits.get(), "{case:?}: the planned window ran");
        assert_eq!(
            (retained, false, false),
            (
                path.exists(),
                log_dir.exists(),
                scratch.path().join("notes.adt.creating").exists()
            ),
            "{case:?}: retain only preexisting files, and publish and leave nothing"
        );
        if retained {
            assert_eq!(
                inode.get(),
                std::fs::metadata(&path).unwrap().ino(),
                "{case:?}"
            );
        }
        assert_eq!(
            b"keep me",
            std::fs::read(&sentinel).unwrap().as_slice(),
            "{case:?}"
        );
        ran += 1;
    }
    assert_eq!(4, ran);
}

#[test]
fn a_stored_body_past_this_builds_limits_refuses_the_open() {
    let scratch = Scratch::new("open-limits", "past");
    let path = scratch.path().join("tall.adt");
    let sums = " + 1".repeat(40);
    let schema = format!(
        "module shop
add module shop
add type Note = {{ id: Key notes, body: Text, n: I64 }}
add relation notes : Note key id generate sequence index (lower(body))
add constraint tall =
  from n in notes
  where n.n{sums} < 0
  select n
"
    );
    let log = Log::new([schema.as_str()]);
    let options = OpenOptions {
        workers: Some(1),
        ..OpenOptions::default()
    };
    let created = OpenOptions {
        create: Some(CreateOptions::default()),
        ..options.clone()
    };
    Database::open(&path, &log, &created)
        .unwrap()
        .close()
        .unwrap();
    adtb_lang::norm::set_run_height(32);
    let got = Database::open(&path, &log, &options).map(drop);
    adtb_lang::norm::set_run_height(64);
    assert_eq!(
        "Err(PastLimits { object: \"shop::tall\" })",
        format!("{got:?}")
    );
}

#[test]
fn a_create_refused_for_a_name_that_is_not_utf8_leaves_no_file() {
    use std::os::unix::ffi::OsStringExt;
    let scratch = Scratch::new("open-acquisition", "not-utf8");
    std::fs::write(scratch.path().join("unrelated"), b"keep me").unwrap();
    let name = std::ffi::OsString::from_vec(b"notes\xff.adt".to_vec());
    let path = scratch.path().join(name);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        ..OpenOptions::default()
    };
    let got = Database::open(&path, &Log::new([NOTES]), &options).map(drop);
    let Err(OpenError::Io(e)) = got else {
        panic!("the create is refused as an I/O error, found {got:?}");
    };
    assert_eq!(
        (
            std::io::ErrorKind::InvalidInput,
            format!("{} names no file in a directory", path.display())
        ),
        (e.kind(), e.to_string())
    );
    let mut left: Vec<_> = std::fs::read_dir(scratch.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    left.sort();
    assert_eq!(vec![std::ffi::OsString::from("unrelated")], left);
    assert_eq!(
        b"keep me",
        std::fs::read(scratch.path().join("unrelated"))
            .unwrap()
            .as_slice()
    );
}
