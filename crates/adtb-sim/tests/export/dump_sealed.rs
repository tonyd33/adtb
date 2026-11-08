//! Dumps encrypted under a passphrase: the round trip, and the refusals of a restore of one
//! damaged, given the wrong passphrase or none.

use adtb::storage::{self, Passphrase};
use adtb::{ArchiveTarget, CallOptions, Database, Error, Log, RejectReason, RestorePoint, Value};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_simkit::search::Files;

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Shape { name: Text } = Circle { r: I64 } | Square { side: I64 }
add relation shapes : Shape key name
";
/// A row value no dump writes but as row data.
const MARK: &str = "zebra-quartz-1729";
const PHRASE: &str = "correct horse battery staple";
const PLAIN: &str = "p";
const SEALED: &str = "e";
const RELATIONS: [&str; 3] = ["app::users", "app::shapes", "catalog::roles"];

/// Returns a device holding `app.adt`, its plaintext dump at [`PLAIN`], its dump encrypted
/// under [`PHRASE`] at [`SEALED`] and its backup at `b`.
fn dumped() -> Files {
    let mut sim = Sim::new(1, Config::default());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, "app.adt", &Log::new([APP]), &open::options(2)).unwrap();
        let rows = format!(
            "transact\n  insert users {{ name: \"{MARK}\" }}\n  insert shapes.Circle {{ name: \"c\", r: 2 }}"
        );
        let key = [("k", Ty::Base(Base::Text), Value::text(PHRASE))];
        for (text, params) in [
            (rows, &[][..]),
            (format!("dump to \"{PLAIN}\""), &[]),
            (format!("dump to \"{SEALED}\" encrypt k"), &key),
            ("backup to \"b\"".to_owned(), &[]),
        ] {
            db.query_text(&text, params, &CallOptions::default())
                .unwrap();
        }
        db.close().unwrap();
    });
    sim.files()
}

/// Returns what a restore of the dump at `source` in `files` under `phrase` answers: the rows
/// of [`RELATIONS`] in the restored file. Asserts a refused restore writes nothing.
fn restored(files: &Files, source: &str, phrase: Option<&str>) -> Result<Vec<String>, Error> {
    let mut sim = Sim::with_files(2, Config::default(), files.clone());
    let got = api::run(&mut sim, |rt| {
        let archive = ArchiveTarget::path(source).unwrap();
        let phrase = phrase.map(|p| Passphrase::new(p.to_owned()));
        let opts = open::options(2);
        let latest = RestorePoint::Latest;
        adtb::restore_report_in(
            rt.clone(),
            (&archive, phrase.as_ref()),
            "c.adt",
            latest,
            &opts,
        )?;
        let db = Database::open_in(rt, "c.adt", &Log::new([APP]), &opts).unwrap();
        let rows = RELATIONS
            .iter()
            .flat_map(|r| api::sorted(api::result(&db, r)));
        let rows = rows.collect();
        db.close().unwrap();
        Ok(rows)
    });
    if got.is_err() {
        assert_eq!(files, &sim.files(), "a refused restore wrote");
    }
    got
}

fn holds(bytes: &[u8], text: &str) -> bool {
    bytes.windows(text.len()).any(|w| w == text.as_bytes())
}

#[test]
fn an_encrypted_dump_restores_the_rows_of_the_plaintext_one_and_holds_no_value_in_clear() {
    let files = dumped();
    let plain = restored(&files, PLAIN, None).unwrap();
    assert!(plain.iter().any(|r| r.contains(MARK)), "{plain:?}");
    assert_eq!(plain, restored(&files, SEALED, Some(PHRASE)).unwrap());
    let within = |dir: &'static str| {
        files
            .iter()
            .filter(move |(p, _)| p.starts_with(&format!("{dir}/")))
    };
    assert!(within(PLAIN).any(|(_, b)| holds(b, MARK)));
    let sealed: Vec<_> = within(SEALED).collect();
    assert!(sealed.len() > 8, "{:?}", files.keys());
    for (path, bytes) in sealed {
        assert!(!holds(bytes, MARK), "{path} holds a row value");
    }
    for (path, bytes) in &files {
        assert!(!holds(bytes, PHRASE), "{path} holds the passphrase");
    }
    let manifest = String::from_utf8_lossy(&files["e/manifest"]).into_owned();
    let keys: Vec<_> = manifest
        .trim_end_matches('\0')
        .lines()
        .skip(1)
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    let clear = "kind format data release edition schema_version schema_hash catalog_version \
        encrypted kdf salt sealed checksum";
    assert_eq!(clear.split(' ').collect::<Vec<_>>(), keys);
}

/// Returns `files` with the encrypted dump's manifest line beginning `key` replaced by `line`, and
/// its checksum written again.
fn edited(files: &Files, key: &str, line: &str) -> Files {
    let bytes = &files["e/manifest"];
    let text = String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_owned();
    let body = &text[..text.rfind("checksum ").unwrap()];
    let old = body.lines().find(|l| l.starts_with(key)).unwrap();
    let body = body.replacen(old, line, 1);
    let mut sealed = format!("{body}checksum {}\n", storage::manifest_hash(&body)).into_bytes();
    sealed.resize(bytes.len(), 0);
    let mut out = files.clone();
    out.insert("e/manifest".to_owned(), sealed);
    out
}

fn damaged(part: &str) -> Result<Vec<String>, Error> {
    Err(Error::Rejected {
        reason: RejectReason::DumpDamaged {
            part: part.to_owned(),
        },
    })
}

#[test]
fn a_wrong_passphrase_or_any_damage_fails_naming_the_first_part_and_writes_nothing() {
    let files = dumped();
    let changed = |edit: &dyn Fn(&mut Files)| {
        let mut out = files.clone();
        edit(&mut out);
        out
    };
    let flipped = changed(&|f| f.get_mut("e/log").unwrap()[40] ^= 1);
    let truncated = changed(&|f| {
        let log = f.get_mut("e/log").unwrap();
        log.truncate(log.len() / 2);
    });
    let swapped = changed(&|f| {
        let ids = f.insert("e/ids".to_owned(), f["e/log"].clone()).unwrap();
        f.insert("e/log".to_owned(), ids);
    });
    let resalted = edited(&files, "salt ", &format!("salt {}", "0".repeat(32)));
    let release = edited(&files, "release ", "release 0.0.0");
    let cases = [
        ("a wrong passphrase", files.clone(), "x", "manifest"),
        ("a flipped byte", flipped, PHRASE, "log"),
        ("a truncated entry", truncated, PHRASE, "log"),
        ("two entries swapped", swapped, PHRASE, "ids"),
        ("a changed salt", resalted, PHRASE, "manifest"),
        ("a changed version field", release, PHRASE, "manifest"),
    ];
    for (label, files, phrase, part) in cases {
        assert_eq!(
            damaged(part),
            restored(&files, SEALED, Some(phrase)),
            "{label}"
        );
    }
}

#[test]
fn a_restore_without_the_passphrase_or_given_one_for_a_plaintext_dump_or_backup_is_invalid() {
    let files = dumped();
    let invalid = |detail: &str| {
        Err(Error::Rejected {
            reason: RejectReason::Invalid {
                detail: detail.to_owned(),
            },
        })
    };
    assert_eq!(
        invalid("the dump is encrypted: its restore needs a passphrase"),
        restored(&files, SEALED, None)
    );
    assert_eq!(
        invalid("the source is not an encrypted dump: its restore takes no passphrase"),
        restored(&files, PLAIN, Some(PHRASE))
    );
    assert_eq!(
        invalid("the source is not an encrypted dump: its restore takes no passphrase"),
        restored(&files, "b", Some(PHRASE))
    );
    assert_eq!(
        invalid("no backup is at `nowhere`"),
        restored(&files, "nowhere", Some(PHRASE))
    );
}
