use std::cell::Cell;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;

use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::storage::{Damage, Options, Pager, Quarantine};

use super::{
    CreateOptions, Database, Log, OpenError, OpenOptions, file, mark_failed_open_damage,
    open_damage,
};

fn unreadable_root(scratch: &Scratch) -> (OsRuntime, Pager<OsRuntime>) {
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let pager = Pager::create(rt.clone(), &Options::new("unreadable.adt")).unwrap();
    drop(Catalog::open(rt.clone(), &pager).unwrap());
    pager.damage(pager.catalog_root(), Damage::Unreadable);
    (rt, pager)
}

#[test]
fn an_unreadable_catalog_root_marks_nothing_and_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-unmarked");
    let (rt, pager) = unreadable_root(&scratch);
    mark_failed_open_damage(rt, &pager);
    assert!(!pager.is_quarantined(Quarantine::Database));
    assert!(open_damage(&pager).is_none());
    pager.close();
}

#[test]
fn an_unreadable_page_on_a_marked_file_opens_nothing() {
    let scratch = Scratch::new("open-damage", "unreadable-marked");
    let (_rt, pager) = unreadable_root(&scratch);
    pager.quarantine(Quarantine::Database).unwrap();
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
