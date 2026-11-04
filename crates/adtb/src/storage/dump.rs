//! Logical dumps: a directory of entries, each listed with its length and BLAKE3 hash in a
//! manifest written last, built aside and published by rename; and the publishing of a database
//! a restore built aside.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::runtime::{IoError, Runtime};

use super::backup::{
    BackupError, Envelope, FIRST_LINE, Fs, Io, KIND as BACKUP, MANIFEST, MAX_EPOCH, Part,
    final_name, hex, inside, io_error, make_parent, name_taken, occupied, publish_restored, sealed,
    unsealed,
};
use super::header::REGION;
use super::pager::parent;
use super::{Lsn, Position};

/// The layout of a dump directory this engine reads and writes.
pub const DUMP_FORMAT: u32 = 1;
const KIND: &str = "dump";
/// The only form of the rows of `data/` this engine writes and reads.
const DATA_FORM: &str = "text";
/// The directories of a dump that hold entries.
const DIRS: [&str; 2] = ["data", "bundles"];

/// What a dump holds and the snapshot it was read at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpManifest {
    pub envelope: Envelope,
    pub snapshot: Position,
    /// Each entry, its `len` the bytes before the zeros that pad its file to a whole region.
    pub parts: Vec<Part>,
}

impl DumpManifest {
    /// Returns the manifest as the text of its file.
    pub fn text(&self) -> String {
        let e = &self.envelope;
        let mut body = String::new();
        for line in [
            FIRST_LINE.to_owned(),
            format!("kind {KIND}"),
            format!("format {DUMP_FORMAT}"),
            format!("data {DATA_FORM}"),
            format!("release {}", e.release),
            format!("edition {}", e.edition),
            format!("schema_version {}", e.schema_version),
            format!("schema_hash {}", e.schema_hash),
            format!("catalog_version {}", e.catalog_version),
            format!("snapshot {}/{}", self.snapshot.epoch, self.snapshot.lsn.0),
            "encrypted 0".to_owned(),
        ] {
            writeln!(body, "{line}").expect("a string takes any text");
        }
        for p in &self.parts {
            writeln!(body, "part {} {} {}", p.name, p.len, p.hash)
                .expect("a string takes any text");
        }
        sealed(body)
    }

    /// Returns the manifest `text` states, its lines in the order [`DumpManifest::text`] writes
    /// them and its parts in ascending order of name.
    fn parse(text: &str) -> Result<DumpManifest, BackupError> {
        let damaged = || BackupError::Damaged {
            part: MANIFEST.to_owned(),
        };
        let mut lines = unsealed(text).ok_or_else(damaged)?.lines();
        if lines.next() != Some(FIRST_LINE) {
            return Err(damaged());
        }
        let mut field = |key: &str| {
            lines
                .next()
                .and_then(|l| l.strip_prefix(key)?.strip_prefix(' '))
                .ok_or_else(damaged)
        };
        if field("kind")? != KIND {
            return Err(damaged());
        }
        match field("format")?.parse() {
            Ok(DUMP_FORMAT) => {}
            Ok(other) => return Err(BackupError::UnknownFormat(other)),
            Err(_) => return Err(damaged()),
        }
        if field("data")? != DATA_FORM {
            return Err(damaged());
        }
        let release = field("release")?.to_owned();
        let edition = field("edition")?.to_owned();
        let schema_version = field("schema_version")?.parse().map_err(|_| damaged())?;
        let schema_hash = field("schema_hash")?.to_owned();
        let catalog_version = field("catalog_version")?.parse().map_err(|_| damaged())?;
        let snapshot = field("snapshot")?;
        let snapshot = snapshot
            .split_once('/')
            .and_then(|(epoch, lsn)| {
                Some(Position {
                    epoch: epoch.parse().ok().filter(|e| (1..=MAX_EPOCH).contains(e))?,
                    lsn: Lsn(lsn.parse().ok()?),
                })
            })
            .ok_or_else(damaged)?;
        if field("encrypted")? != "0" {
            return Err(damaged());
        }
        let parts: Vec<Part> = lines
            .map(|l| {
                let mut words = l.strip_prefix("part ")?.rsplitn(3, ' ');
                let (hash, len, name) = (words.next()?, words.next()?, words.next()?);
                Some(Part {
                    name: name.to_owned(),
                    len: len.parse().ok()?,
                    hash: hash.to_owned(),
                })
            })
            .collect::<Option<_>>()
            .filter(|ps: &Vec<Part>| ps.iter().all(|p| entry_name(&p.name)))
            .filter(|ps| ps.windows(2).all(|w| w[0].name < w[1].name))
            .ok_or_else(damaged)?;
        Ok(DumpManifest {
            envelope: Envelope {
                release,
                edition,
                schema_version,
                schema_hash,
                catalog_version,
            },
            snapshot,
            parts,
        })
    }
}

/// Returns whether `name` is a path a dump may hold an entry at: a name, or one inside a
/// directory of [`DIRS`].
fn entry_name(name: &str) -> bool {
    let plain = |n: &str| !n.is_empty() && !n.contains('/') && n != "." && n != "..";
    match name.split_once('/') {
        Some((dir, rest)) => DIRS.contains(&dir) && plain(rest),
        None => plain(name),
    }
}

/// Returns whether the manifest at `source` is a dump's: a sealed one whose second line names a
/// dump, or one whose seal fails and whose second line does not name a backup.
pub fn is_dump<R: Runtime>(rt: &R, source: &str) -> Io<bool> {
    let mut fs = Fs::new(rt.clone());
    let Some(text) = fs.read_text(&inside(source, MANIFEST))? else {
        return Ok(false);
    };
    let line = |kind: &str| format!("kind {kind}");
    Ok(match unsealed(&text) {
        Some(body) => body.lines().nth(1) == Some(&line(KIND)),
        None => text.lines().nth(1) != Some(&line(BACKUP)),
    })
}

/// Write `entries`, each a path inside the dump and its bytes, to the absent or empty directory
/// `target`, the manifest last.
///
/// Postconditions:
/// - Until the dump is complete, sealed and forced, nothing exists at the target.
/// - On a failure before the rename, the target is as it was and the private copy removed.
pub fn write_dump<R: Runtime>(
    rt: &R,
    target: &str,
    envelope: Envelope,
    snapshot: Position,
    entries: &[(String, Vec<u8>)],
) -> Result<DumpManifest, BackupError> {
    let mut fs = Fs::new(rt.clone());
    let target = final_name(target);
    let taken = || BackupError::Occupied(target.to_owned());
    occupied(&mut fs, target, taken)?;
    make_parent(&mut fs, target, taken)?;
    let aside = fs.aside_dir(target)?;
    let done = write_entries(&mut fs, &aside, entries).and_then(|mut parts| {
        parts.sort_by(|a, b| a.name.cmp(&b.name));
        let manifest = DumpManifest {
            envelope,
            snapshot,
            parts,
        };
        let path = inside(&aside, MANIFEST);
        let file = fs.create(&path)?;
        fs.write_text(&path, file, &manifest.text())?;
        fs.sync_dir(&aside)?;
        fs.publish(&aside, target, taken)?;
        Ok(manifest)
    });
    if done.is_err() {
        fs.discard(&aside).ok();
    }
    done
}

fn write_entries<R: Runtime>(
    fs: &mut Fs<R>,
    aside: &str,
    entries: &[(String, Vec<u8>)],
) -> Io<Vec<Part>> {
    for dir in DIRS {
        fs.make_dirs(&inside(aside, dir), || {
            BackupError::Occupied(aside.to_owned())
        })?;
    }
    let mut parts = Vec::with_capacity(entries.len());
    for (name, bytes) in entries {
        let path = inside(aside, name);
        let file = fs.create(&path)?;
        let mut padded = bytes.clone();
        padded.resize(bytes.len().next_multiple_of(REGION), 0);
        fs.closing(&path, file, |fs| {
            fs.write(&path, file, 0, &padded)?;
            fs.sync(&path, file)
        })?;
        parts.push(Part {
            name: name.clone(),
            len: bytes.len() as u64,
            hash: hex(blake3::hash(bytes).as_bytes()),
        });
    }
    for dir in DIRS {
        fs.sync_dir(&inside(aside, dir))?;
    }
    Ok(parts)
}

/// Returns the manifest of the dump at `source` and the bytes of each entry it lists, once
/// every entry checks against its length and hash, in the manifest's order.
pub fn read_dump<R: Runtime>(
    rt: &R,
    source: &str,
) -> Result<(DumpManifest, BTreeMap<String, Vec<u8>>), BackupError> {
    let mut fs = Fs::new(rt.clone());
    let text = fs
        .read_text(&inside(source, MANIFEST))?
        .ok_or_else(|| BackupError::Absent(source.to_owned()))?;
    let manifest = DumpManifest::parse(&text)?;
    let mut entries = BTreeMap::new();
    for part in &manifest.parts {
        let bytes = read_entry(&mut fs, source, part)?;
        entries.insert(part.name.clone(), bytes);
    }
    Ok((manifest, entries))
}

fn read_entry<R: Runtime>(fs: &mut Fs<R>, source: &str, part: &Part) -> Io<Vec<u8>> {
    let damaged = || BackupError::Damaged {
        part: part.name.clone(),
    };
    let path = inside(source, &part.name);
    let (file, len) = match fs.drv.open(&path, false) {
        Ok(opened) => opened,
        Err(IoError::NotFound | IoError::NotADirectory) => return Err(damaged()),
        Err(e) => return Err(io_error(&path)(e)),
    };
    let whole = part
        .len
        .checked_next_multiple_of(REGION as u64)
        .filter(|&n| n == len)
        .and_then(|n| usize::try_from(n).ok());
    let bytes = fs.closing(&path, file, |fs| match whole {
        Some(0) => Ok(None),
        Some(whole) => fs.read(&path, file, 0, whole).map(Some),
        None => Err(damaged()),
    })?;
    let bytes = bytes.as_ref().map_or(&[][..], |b| &b[..]);
    let (body, pad) = bytes.split_at(part.len as usize);
    if pad.iter().any(|&b| b != 0) || hex(blake3::hash(body).as_bytes()) != part.hash {
        return Err(damaged());
    }
    Ok(body.to_vec())
}

/// Returns a private name beside `path` for a restore to build a database at, once `path` is
/// free and `log_dir` absent or empty, and the directories it made above `path`, outermost first.
pub fn restore_aside<R: Runtime>(rt: &R, path: &str, log_dir: &str) -> Io<(String, Vec<String>)> {
    let mut fs = Fs::new(rt.clone());
    let (path, log_dir) = (final_name(path), final_name(log_dir));
    let exists = || BackupError::Exists(path.to_owned());
    name_taken(&mut fs, path, exists)?;
    occupied(&mut fs, log_dir, || BackupError::Exists(log_dir.to_owned()))?;
    let made = make_parent(&mut fs, path, exists)?;
    Ok((fs.aside_name(path), made))
}

/// Remove each of `made`, innermost first, while it is empty.
fn remove_made<R: Runtime>(fs: &mut Fs<R>, made: &[String]) {
    for dir in made.iter().rev() {
        if fs.remove_dir(dir).is_err() {
            return;
        }
        fs.sync_dir(parent(dir)).ok();
    }
}

/// Move the closed database at `aside`, with its log directory `aside_log`, to `path` and
/// `log_dir`, the file's name last.
///
/// Postcondition: on a failure before the file's rename, `path` is as it was.
pub fn publish_database<R: Runtime>(
    rt: &R,
    (aside, aside_log): (&str, &str),
    path: &str,
    log_dir: &str,
) -> Io<()> {
    let mut fs = Fs::new(rt.clone());
    publish_restored(
        &mut fs,
        aside,
        final_name(path),
        aside_log,
        final_name(log_dir),
    )
}

/// Remove each of `paths`, files or directories a restore made under private names, then each
/// directory of `made` while it is empty, innermost first.
pub fn discard_aside<R: Runtime>(rt: &R, paths: &[&str], made: &[String]) {
    let mut fs = Fs::new(rt.clone());
    for path in paths {
        fs.discard(path).ok();
    }
    remove_made(&mut fs, made);
}
