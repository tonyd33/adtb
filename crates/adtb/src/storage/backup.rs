//! Full backups: a copy of the data file's pages taken under live writers, the log from the redo
//! point of a checkpoint to an end position, a manifest written last. Restoring one replays that
//! log over the copy, as recovery after a crash at the end position would.
//!
//! A run builds under a private name beside its destination, and renames last.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::runtime::{AlignedBuf, Done, FileId, IoError, Op, Runtime};

use super::block::{
    Batch, BlockState, LogImage, LogProblem, LogSource, Mode, Slot, read_block, scan,
};
use super::digest::{hex, sealed, unhex, unsealed};
use super::driver::Driver;
use super::header::{FileHeader, HISTORY_CAPACITY, REGION};
use super::log::{FileSource, parse_segment, segment_name};
use super::page::{self, spoil, valid_page_size};
use super::pager::{Damage, Pager, parent};
use super::quarantine::Quarantine;
use super::record::Record;
use super::{Lsn, Pid, Position};

/// The layout of a backup directory this engine reads and writes.
pub const FORMAT: u32 = 1;
/// The kind of a directory holding a physical copy.
pub(super) const KIND: &str = "backup";
pub(super) const FIRST_LINE: &str = "adtb manifest";
const PROOF_LINE: &str = "adtb proof";

/// The file a backup is complete once it holds in full.
pub const MANIFEST: &str = "manifest";
/// The file recording a test restore's result.
pub const PROOF: &str = "proof";
/// The stem of the private directory a test restore rebuilds the database in.
const SCRATCH: &str = "scratch";
/// The smallest block a log writes.
const MIN_LOG_BLOCK: u64 = 512;
/// What joins a final name and the random tail of its private name.
pub(super) const PARTIAL: &str = ".partial-";

const PAGES: &str = "pages";
const LOG: &str = "log";

pub(super) const IO_CHUNK: u64 = 1 << 20;
/// The page ids the first [`REGION`] bytes of a chunk of a page part hold, after their count.
const CHUNK_PAGES: usize = REGION / 8 - 1;
/// The chunks of one page part.
const PART_CHUNKS: usize = 64;
/// The most a restore raises the epoch by.
pub(super) const EPOCH_STEP: u64 = 1 << 32;
/// The highest epoch a manifest may end in: a restore of it stays within an `I64`.
pub(super) const MAX_EPOCH: u64 = (1 << 63) - 1 - EPOCH_STEP;

/// What a backup says of the database beyond its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub release: String,
    pub edition: String,
    pub schema_version: u64,
    pub schema_hash: String,
    pub catalog_version: u64,
}

/// A file of a backup, by its path inside the backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub len: u64,
    pub hash: String,
}

/// What a backup holds and which state of the database restoring it yields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub envelope: Envelope,
    /// The source's header, naming the backup's checkpoint.
    pub header: FileHeader,
    pub file_pages: u64,
    pub start: Position,
    pub end: Position,
    /// The wall instant the copy ended, in microseconds.
    pub end_time: i64,
    /// The backup this one builds on.
    pub parent: Option<String>,
    pub parts: Vec<Part>,
    pub damaged: Vec<Pid>,
    pub quarantined: Vec<String>,
}

/// Why a backup or restore stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    /// The target of a backup holds files.
    Occupied(String),
    /// The path a restore would write exists.
    Exists(String),
    /// The caller's visit asked to stop.
    Stopped,
    /// The database became quarantined as a whole.
    Quarantined,
    /// No manifest is at the path a restore or proof reads.
    Absent(String),
    /// A part fails its check.
    Damaged {
        part: String,
    },
    /// The manifest is whole and written in a layout this engine does not read.
    UnknownFormat(u32),
    /// The manifest is whole and describes something other than a backup.
    OtherKind(String),
    /// The dump is encrypted and the restore has no passphrase, or the reverse.
    KeyMismatch {
        encrypted: bool,
    },
    Io {
        what: String,
        error: IoError,
    },
}

/// One run of a backup.
pub struct BackupRun<'a> {
    pub target: &'a str,
    pub envelope: Envelope,
    /// Called with each page as first read, unlatched and empty when the read failed, and with
    /// `Pid(0)` and nothing before each stretch of the log. Returning false stops the backup.
    pub visit: &'a mut dyn FnMut(Pid, &mut [u8]) -> bool,
    /// Returns the name a client sees of a quarantined object.
    pub name: &'a dyn Fn(Quarantine) -> String,
}

impl std::fmt::Debug for BackupRun<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupRun")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// File operations that report a failure to their caller and name the file.
pub(super) struct Fs<R: Runtime> {
    pub(super) drv: Driver<R>,
}

pub(super) type Io<T> = Result<T, BackupError>;

pub(super) fn io_error(what: impl Into<String>) -> impl FnOnce(IoError) -> BackupError {
    let what = what.into();
    move |error| BackupError::Io { what, error }
}

impl<R: Runtime> Fs<R> {
    pub(super) fn new(rt: R) -> Fs<R> {
        Fs {
            drv: Driver::new(rt),
        }
    }

    pub(super) fn open(&mut self, path: &str, create: bool) -> Io<(FileId, u64)> {
        self.drv.open(path, create).map_err(io_error(path))
    }

    pub(super) fn create(&mut self, path: &str) -> Io<FileId> {
        let (file, _) = self.drv.create_new(path).map_err(io_error(path))?;
        Ok(file)
    }

    /// Returns a private name beside `name`.
    pub(super) fn aside_name(&self, name: &str) -> String {
        format!("{name}{PARTIAL}{:016x}", self.drv.rt.next_u64())
    }

    /// Create a new private directory beside `name` and return its path.
    pub(super) fn aside_dir(&mut self, name: &str) -> Io<String> {
        loop {
            let dir = self.aside_name(name);
            let op = Op::CreateDir { dir: dir.clone() };
            match self.drv.attempt(op).result {
                Ok(Done::Created) => return Ok(dir),
                Ok(_) | Err(IoError::Exists) => {}
                Err(e) => return Err(io_error(dir)(e)),
            }
        }
    }

    /// Create a new private file beside `name` and return its path and id.
    pub(super) fn aside_file(&mut self, name: &str) -> Io<(String, FileId)> {
        loop {
            let path = self.aside_name(name);
            match self.drv.create_new(&path) {
                Ok((file, _)) => return Ok((path, file)),
                Err(IoError::Exists) => {}
                Err(e) => return Err(io_error(path)(e)),
            }
        }
    }

    /// Rename `from` to `to`. A taken name answers `taken`.
    pub(super) fn rename(
        &mut self,
        from: &str,
        to: &str,
        taken: impl FnOnce() -> BackupError,
    ) -> Io<()> {
        let op = Op::Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            replace: false,
        };
        match self.drv.attempt(op).result {
            Ok(_) => Ok(()),
            Err(IoError::Exists | IoError::NotADirectory) => Err(taken()),
            Err(e) => Err(io_error(to)(e)),
        }
    }

    /// Force `aside`'s name, rename it to `to`, and force that.
    pub(super) fn publish(
        &mut self,
        aside: &str,
        to: &str,
        taken: impl FnOnce() -> BackupError,
    ) -> Io<()> {
        self.sync_dir(parent(to))?;
        self.rename(aside, to, taken)?;
        self.sync_dir(parent(to))
    }

    /// Remove the private file or directory `path` and all under it, going on past a failed
    /// removal, and force its name's removal. Returns the first failure.
    pub(super) fn discard(&mut self, path: &str) -> Io<()> {
        let removed = self.remove_tree(path);
        removed.and(self.sync_dir(parent(path)))
    }

    fn remove_tree(&mut self, path: &str) -> Io<()> {
        let entries = match self.list(path) {
            Ok(entries) => entries,
            Err(BackupError::Io {
                error: IoError::NotADirectory,
                ..
            }) => return self.remove(path),
            Err(e) => return Err(e),
        };
        let mut out = Ok(());
        for entry in entries {
            out = out.and(self.remove_tree(&entry));
        }
        out.and(self.remove_dir(path))
    }

    /// Run `body` and close `file`, whatever `body` answers.
    pub(super) fn closing<T>(
        &mut self,
        path: &str,
        file: FileId,
        body: impl FnOnce(&mut Self) -> Io<T>,
    ) -> Io<T> {
        let out = body(self);
        let closed = self.close(path, file);
        let out = out?;
        closed?;
        Ok(out)
    }

    fn run(&mut self, what: &str, op: Op) -> Io<Done> {
        self.drv.attempt(op).result.map_err(io_error(what))
    }

    pub(super) fn write(&mut self, what: &str, file: FileId, offset: u64, bytes: &[u8]) -> Io<()> {
        let buf = AlignedBuf::from_slice(bytes);
        self.run(what, Op::Write { file, offset, buf }).map(|_| ())
    }

    pub(super) fn sync(&mut self, what: &str, file: FileId) -> Io<()> {
        self.run(what, Op::Sync { file }).map(|_| ())
    }

    pub(super) fn sync_dir(&mut self, dir: &str) -> Io<()> {
        let dir = dir.to_owned();
        self.run(&dir.clone(), Op::SyncDir { dir }).map(|_| ())
    }

    pub(super) fn list(&mut self, dir: &str) -> Io<Vec<String>> {
        let op = Op::List {
            dir: dir.to_owned(),
        };
        match self.drv.attempt(op).result {
            Ok(Done::Listed(paths)) => Ok(paths),
            Ok(other) => unreachable!("list completed as {other:?}"),
            Err(IoError::NotFound) => Ok(Vec::new()),
            Err(error) => Err(BackupError::Io {
                what: dir.to_owned(),
                error,
            }),
        }
    }

    /// Create `dir` and every absent directory above it, and force the name of each one made. An
    /// entry of another kind on the way answers `taken`.
    /// Make `dir` and each directory above it that is absent. Returns the ones it made, outermost
    /// first.
    pub(super) fn make_dirs(
        &mut self,
        dir: &str,
        taken: impl Fn() -> BackupError,
    ) -> Io<Vec<String>> {
        let mut made = Vec::new();
        let mut at = String::new();
        for (i, name) in dir.split('/').enumerate() {
            if i > 0 {
                at.push('/');
            }
            at.push_str(name);
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let path = at.clone();
            match self.drv.attempt(Op::CreateDir { dir: path }).result {
                Ok(Done::Created) => {
                    made.push(at.clone());
                    self.sync_dir(parent(&at))?;
                }
                Ok(_) => {}
                Err(IoError::Exists | IoError::NotADirectory) => return Err(taken()),
                Err(e) => return Err(io_error(at)(e)),
            }
        }
        Ok(made)
    }

    pub(super) fn remove(&mut self, path: &str) -> Io<()> {
        let op = Op::Remove {
            path: path.to_owned(),
        };
        match self.drv.attempt(op).result {
            Ok(_) | Err(IoError::NotFound) => Ok(()),
            Err(error) => Err(BackupError::Io {
                what: path.to_owned(),
                error,
            }),
        }
    }

    pub(super) fn remove_dir(&mut self, path: &str) -> Io<()> {
        let op = Op::RemoveDir {
            path: path.to_owned(),
        };
        match self.drv.attempt(op).result {
            Ok(_) | Err(IoError::NotFound) => Ok(()),
            Err(error) => Err(BackupError::Io {
                what: path.to_owned(),
                error,
            }),
        }
    }

    fn close(&mut self, what: &str, file: FileId) -> Io<()> {
        self.run(what, Op::Close { file }).map(|_| ())
    }

    pub(super) fn read(&mut self, what: &str, file: FileId, at: u64, len: usize) -> Io<AlignedBuf> {
        self.drv.read(file, at, len).map_err(io_error(what))
    }

    /// Write `text` to the open file `file`, zero to a multiple of [`REGION`], force it and close
    /// it, and force its name.
    pub(super) fn write_text(&mut self, path: &str, file: FileId, text: &str) -> Io<()> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(bytes.len().next_multiple_of(REGION), 0);
        self.closing(path, file, |fs| {
            fs.write(path, file, 0, &bytes)?;
            fs.sync(path, file)
        })?;
        self.sync_dir(parent(path))
    }

    /// Returns the text of `path` up to its first zero byte, `None` when the file is absent.
    pub(super) fn read_text(&mut self, path: &str) -> Io<Option<String>> {
        let (file, len) = match self.drv.open(path, false) {
            Ok(opened) => opened,
            Err(IoError::NotFound | IoError::NotADirectory) => return Ok(None),
            Err(e) => return Err(io_error(path)(e)),
        };
        let whole = usize::try_from(len).unwrap_or(0).next_multiple_of(REGION);
        let bytes = self.closing(path, file, |fs| fs.read(path, file, 0, whole))?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(Some(String::from_utf8_lossy(&bytes[..end]).into_owned()))
    }
}

/// Returns `dir/name`, or `name` where `dir` is empty.
pub(super) fn inside(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Returns the hash naming a manifest.
pub fn manifest_hash(text: &str) -> String {
    hex(blake3::hash(text.as_bytes()).as_bytes())
}

impl Manifest {
    /// Returns the manifest as the text of its file.
    pub fn text(&self) -> String {
        sealed(self.lines().join("\n") + "\n")
    }

    fn lines(&self) -> Vec<String> {
        let (e, h) = (&self.envelope, &self.header);
        let mut lines = vec![
            FIRST_LINE.to_owned(),
            format!("kind {KIND}"),
            format!("format {FORMAT}"),
            format!("release {}", e.release),
            format!("edition {}", e.edition),
            format!("page_formats {} {}", h.lowest_format, h.highest_format),
            format!("upgrade_state {}", h.upgrade_state),
            format!("schema_version {}", e.schema_version),
            format!("schema_hash {}", e.schema_hash),
            format!("catalog_version {}", e.catalog_version),
            format!("file_id {}", hex(&h.file_id)),
            format!("page_size {}", h.page_size),
            format!("log_block {}", h.log_block),
            format!("segment_size {}", h.segment_size),
            format!("atomic_page_writes {}", u8::from(h.atomic_page_writes)),
            format!("encryption {}", h.encryption),
            format!("catalog_root {}", h.catalog_root.0),
            format!("quarantine_page {}", h.quarantine.0),
            format!("bitmap_groups {}", h.bitmap_groups),
            format!("file_pages {}", self.file_pages),
            format!("start {}/{}", self.start.epoch, self.start.lsn.0),
            format!("checkpoint {}", h.checkpoint.0),
            format!("end {}/{}", self.end.epoch, self.end.lsn.0),
            format!("end_time {}", self.end_time),
        ];
        for (epoch, end) in &h.history {
            lines.push(format!("epoch_ended {epoch} {}", end.0));
        }
        if let Some(parent) = &self.parent {
            lines.push(format!("parent {parent}"));
        }
        for p in &self.parts {
            lines.push(format!("part {} {} {}", p.name, p.len, p.hash));
        }
        for pid in &self.damaged {
            lines.push(format!("damaged {}", pid.0));
        }
        for q in &self.quarantined {
            lines.push(format!("quarantined {q}"));
        }
        lines
    }

    /// Returns the manifest `text` holds, or the failure of reading it.
    fn parse(text: &str) -> Result<Manifest, BackupError> {
        let damaged = || BackupError::Damaged {
            part: MANIFEST.to_owned(),
        };
        let lines = unsealed(text).ok_or_else(damaged)?;
        if lines.lines().next() != Some(FIRST_LINE) {
            return Err(damaged());
        }
        let found = |key: &str| {
            lines
                .lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
        };
        match found("format").map(str::parse) {
            Some(Ok(FORMAT)) => {}
            Some(Ok(other)) => return Err(BackupError::UnknownFormat(other)),
            _ => return Err(damaged()),
        }
        match found("kind") {
            Some(KIND) => {}
            Some(other) => return Err(BackupError::OtherKind(other.to_owned())),
            None => return Err(damaged()),
        }
        Manifest::read_lines(lines)
            .filter(Manifest::in_bounds)
            .ok_or_else(damaged)
    }

    /// Returns whether every field holds a value a backup of this engine can hold.
    fn in_bounds(&self) -> bool {
        let h = &self.header;
        let block = u64::from(h.log_block);
        let log_fits = valid_page_size(h.page_size)
            && block >= MIN_LOG_BLOCK
            && block.is_power_of_two()
            && h.segment_size > 0
            && h.segment_size.is_multiple_of(block);
        let parts_fit = self.parts.iter().all(|p| {
            let named = |dir| {
                p.name.strip_prefix(dir).is_some_and(|rest: &str| {
                    !rest.is_empty() && !rest.contains('/') && rest != "." && rest != ".."
                })
            };
            let log = named("log/");
            (log || named("pages/")) && (!log || p.len <= h.segment_size)
        });
        let pid_fits = |pid: Pid| pid.0 < self.file_pages;
        log_fits
            && parts_fit
            && h.lowest_format <= h.highest_format
            && self.file_pages >= Pid::FIRST_DATA.0
            && pid_fits(h.catalog_root)
            && pid_fits(h.quarantine)
            && self.damaged.iter().copied().all(pid_fits)
            && self.start.epoch <= self.end.epoch
            && self.start.lsn <= h.checkpoint
            && h.checkpoint < self.end.lsn
            && h.history.len() <= HISTORY_CAPACITY
            && h.history.windows(2).all(|w| w[0].0 < w[1].0)
    }

    fn read_lines(lines: &str) -> Option<Manifest> {
        let found = |key: &str| {
            lines
                .lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
        };
        let all = |key: &'static str| {
            lines
                .lines()
                .filter_map(move |l| l.strip_prefix(key)?.strip_prefix(' '))
        };
        let number = |key: &str| found(key)?.parse::<u64>().ok();
        let small = |key: &str| found(key)?.parse::<u32>().ok();
        let position = |key: &str| {
            let (epoch, lsn) = found(key)?.split_once('/')?;
            Some(Position {
                epoch: epoch.parse().ok().filter(|&e| e <= MAX_EPOCH)?,
                lsn: Lsn(lsn.parse().ok()?),
            })
        };
        let (lowest, highest) = found("page_formats")?.split_once(' ')?;
        let mut parts = Vec::new();
        for p in all("part") {
            let mut words = p.split(' ');
            parts.push(Part {
                name: words.next()?.to_owned(),
                len: words.next()?.parse().ok()?,
                hash: words.next()?.to_owned(),
            });
        }
        let mut history = Vec::new();
        for h in all("epoch_ended") {
            let (epoch, end) = h.split_once(' ')?;
            history.push((epoch.parse().ok()?, Lsn(end.parse().ok()?)));
        }
        let mut damaged = Vec::new();
        for d in all("damaged") {
            damaged.push(Pid(d.parse().ok()?));
        }
        let start = position("start")?;
        let header = FileHeader {
            seq: 0,
            file_id: unhex(found("file_id")?)?,
            page_size: small("page_size")?,
            log_block: small("log_block")?,
            segment_size: number("segment_size")?,
            lowest_format: lowest.parse().ok()?,
            highest_format: highest.parse().ok()?,
            upgrade_state: found("upgrade_state")?.parse().ok()?,
            atomic_page_writes: found("atomic_page_writes")? == "1",
            checkpoint: Lsn(number("checkpoint")?),
            catalog_root: Pid(number("catalog_root")?),
            quarantine: Pid(number("quarantine_page")?),
            epoch: start.epoch,
            encryption: small("encryption")?,
            bitmap_groups: number("bitmap_groups")?,
            history,
            log_start: Lsn(0),
        };
        Some(Manifest {
            envelope: Envelope {
                release: found("release")?.to_owned(),
                edition: found("edition")?.to_owned(),
                schema_version: number("schema_version")?,
                schema_hash: found("schema_hash")?.to_owned(),
                catalog_version: number("catalog_version")?,
            },
            header,
            file_pages: number("file_pages")?,
            start,
            end: position("end")?,
            end_time: found("end_time")?.parse().ok()?,
            parent: found("parent").map(str::to_owned),
            parts,
            damaged,
            quarantined: all("quarantined").map(str::to_owned).collect(),
        })
    }
}

/// The parts under `pages/` being written.
struct PageWriter<'f, R: Runtime> {
    fs: &'f mut Fs<R>,
    target: String,
    parts: Vec<Part>,
    open: Option<OpenPart>,
    pids: Vec<Pid>,
    pages: Vec<u8>,
}

struct OpenPart {
    name: String,
    file: FileId,
    at: u64,
    hash: blake3::Hasher,
    chunks: usize,
}

impl<R: Runtime> PageWriter<'_, R> {
    fn push(&mut self, pid: Pid, page: &[u8]) -> Io<()> {
        self.pids.push(pid);
        self.pages.extend_from_slice(page);
        if self.pids.len() == CHUNK_PAGES {
            self.write_chunk()?;
        }
        Ok(())
    }

    fn write_chunk(&mut self) -> Io<()> {
        let Some(&first) = self.pids.first() else {
            return Ok(());
        };
        if self.open.is_none() {
            let name = inside(PAGES, &segment_name(first.0));
            let file = self.fs.create(&inside(&self.target, &name))?;
            self.open = Some(OpenPart {
                name,
                file,
                at: 0,
                hash: blake3::Hasher::new(),
                chunks: 0,
            });
        }
        let mut chunk = vec![0; REGION];
        chunk[..8].copy_from_slice(&(self.pids.len() as u64).to_le_bytes());
        for (i, pid) in self.pids.iter().enumerate() {
            chunk[8 + i * 8..16 + i * 8].copy_from_slice(&pid.0.to_le_bytes());
        }
        chunk.append(&mut self.pages);
        self.pids.clear();
        let part = self.open.as_mut().expect("opened above");
        part.hash.update(&chunk);
        part.chunks += 1;
        let (name, file, at) = (part.name.clone(), part.file, part.at);
        part.at += chunk.len() as u64;
        self.fs.write(&name, file, at, &chunk)?;
        if self.open.as_ref().is_some_and(|p| p.chunks == PART_CHUNKS) {
            self.end_part()?;
        }
        Ok(())
    }

    fn end_part(&mut self) -> Io<()> {
        if let Some(part) = self.open.take() {
            let (name, file) = (part.name.clone(), part.file);
            self.fs.closing(&name, file, |fs| fs.sync(&name, file))?;
            self.parts.push(Part {
                name: part.name,
                len: part.at,
                hash: hex(part.hash.finalize().as_bytes()),
            });
        }
        Ok(())
    }

    /// Close the part being written after a failure.
    fn abandon(&mut self) -> Io<()> {
        match self.open.take() {
            Some(part) => self.fs.close(&part.name, part.file),
            None => Ok(()),
        }
    }

    /// Write what is pending and return the parts written.
    fn finish(mut self) -> Io<Vec<Part>> {
        let written = self.write_chunk().and_then(|()| self.end_part());
        if let Err(e) = written {
            self.abandon().ok();
            return Err(e);
        }
        Ok(self.parts)
    }
}

/// Returns the bytes of a page the device refuses to read: they fail verification wherever they
/// are read back.
fn unreadable_page(size: usize) -> Vec<u8> {
    vec![0xff; size]
}

impl<R: Runtime> Pager<R> {
    /// Copy the database to the absent or empty directory `run.target` while writers run.
    ///
    /// Postconditions:
    /// - Until the copy is complete, sealed and forced, nothing exists at the target.
    /// - Restoring the backup yields the database as of the returned manifest's end position.
    /// - On a failure before the rename, the target is as it was and the private copy removed. A
    ///   directory made above the target stays.
    /// - A target another run got to first answers `Occupied`.
    pub fn backup(&self, run: &mut BackupRun<'_>) -> Result<Manifest, BackupError> {
        let mut fs = Fs::new(self.rt.clone());
        let target = final_name(run.target).to_owned();
        let taken = || BackupError::Occupied(target.clone());
        occupied(&mut fs, &target, taken)?;
        make_parent(&mut fs, &target, taken)?;
        let aside = fs.aside_dir(&target)?;
        let done = self.build_aside(&mut fs, run, &aside).and_then(|manifest| {
            fs.publish(&aside, &target, taken)?;
            Ok(manifest)
        });
        if done.is_err() {
            fs.discard(&aside).ok();
        }
        done
    }

    /// Write the whole backup in the private directory `aside` and force it.
    fn build_aside(
        &self,
        fs: &mut Fs<R>,
        run: &mut BackupRun<'_>,
        aside: &str,
    ) -> Result<Manifest, BackupError> {
        fs.make_dirs(&inside(aside, PAGES), || read_failed(Lsn(0)))?;
        fs.make_dirs(&inside(aside, LOG), || read_failed(Lsn(0)))?;
        let manifest = self.copy_to(fs, run, aside)?;
        let path = inside(aside, MANIFEST);
        let file = fs.create(&path)?;
        fs.write_text(&path, file, &manifest.text())?;
        Ok(manifest)
    }

    fn copy_to(
        &self,
        fs: &mut Fs<R>,
        run: &mut BackupRun<'_>,
        aside: &str,
    ) -> Result<Manifest, BackupError> {
        self.flush_pages(usize::MAX);
        let (at, redo, groups) = self.log_checkpoint();
        let (mut parts, damaged) = self.copy_pages(fs, run, aside)?;
        if self.is_quarantined(Quarantine::Database) {
            return Err(BackupError::Quarantined);
        }
        let header = self.taken_header(at, groups);
        self.flush_log();
        let end = self.durable();
        parts.extend(self.copy_log(fs, run, aside, redo, end)?);
        let mut quarantined: Vec<String> = self.quarantined().into_iter().map(run.name).collect();
        quarantined.sort();
        Ok(Manifest {
            envelope: run.envelope.clone(),
            file_pages: self.file_pages(),
            start: self.position(redo),
            end: self.position(end),
            end_time: wall_micros(self.rt.wall()),
            parent: None,
            parts,
            damaged: damaged.into_iter().collect(),
            quarantined,
            header,
        })
    }

    /// Returns the header a restore starts from. It names the backup's checkpoint, whatever later
    /// one the file names, whose redo point would skip the repair of pages copied before it.
    fn taken_header(&self, checkpoint: Lsn, groups: u64) -> FileHeader {
        let mut header = self.header.lock(&self.rt).0.clone();
        header.seq = 0;
        header.checkpoint = checkpoint;
        header.bitmap_groups = groups;
        header
    }

    fn copy_pages(
        &self,
        fs: &mut Fs<R>,
        run: &mut BackupRun<'_>,
        aside: &str,
    ) -> Result<(Vec<Part>, BTreeSet<Pid>), BackupError> {
        let mut writer = PageWriter {
            fs,
            target: aside.to_owned(),
            parts: Vec::new(),
            open: None,
            pids: Vec::new(),
            pages: Vec::with_capacity(CHUNK_PAGES * self.page_size),
        };
        let mut damaged = BTreeSet::new();
        for n in Pid::FIRST_DATA.0..self.file_pages() {
            let pid = Pid(n);
            let copied = self
                .page_as_stored(pid, run.visit, &mut damaged)
                .and_then(|page| writer.push(pid, &page));
            if let Err(e) = copied {
                writer.abandon().ok();
                return Err(e);
            }
        }
        let parts = writer.finish()?;
        Ok((parts, damaged))
    }

    /// Returns the bytes of page `pid` for the copy. A read that fails verification, of a page this
    /// open refused as stale, or that the pool holds at a later LSN may have met a write in flight,
    /// so it is read again under the exclusive latch, which no write-out holds with. A page still
    /// failing is added to `damaged` and copied as stored with a checksum it fails, so a
    /// restore refuses it whatever its LSN, unless the allocation bitmap reads and calls it free or
    /// the pool's changed copy is to overwrite it. Only such a page is added.
    fn page_as_stored(
        &self,
        pid: Pid,
        visit: &mut dyn FnMut(Pid, &mut [u8]) -> bool,
        damaged: &mut BTreeSet<Pid>,
    ) -> Result<Vec<u8>, BackupError> {
        let mapped = self.mapped(pid).map_err(|_| BackupError::Quarantined)?;
        let first = self.read_disk(pid);
        let bytes = match first {
            Ok((mut buf, _)) => {
                if !visit(pid, &mut buf) {
                    return Err(BackupError::Stopped);
                }
                let held = self.mem.lock_shared(pid.0, &self.rt);
                if self.trusted(pid, &buf, held.as_deref(), mapped) {
                    return Ok(buf.to_vec());
                }
                Some(buf)
            }
            Err(_) => {
                if !visit(pid, &mut []) {
                    return Err(BackupError::Stopped);
                }
                None
            }
        };
        // Reading the bitmap under this latch can deadlock with a writer latching in page order.
        let free = matches!(self.is_free(pid), Ok(true));
        let g = self.mem.lock_exclusive(pid.0, &self.rt);
        let again = self.read_disk(pid);
        if let Ok((buf, _)) = &again
            && self.trusted(pid, buf, g.resident().then(|| &*g), mapped)
        {
            return Ok(buf.to_vec());
        }
        if g.resident() && self.meta().changed.contains(&pid) {
            return Ok(self.staged(pid, &g).to_vec());
        }
        let mut copy = match (again, bytes) {
            (Ok((buf, _)), _) | (Err(_), Some(buf)) => buf.to_vec(),
            (Err(_), None) => unreadable_page(self.page_size),
        };
        if !free {
            damaged.insert(pid);
            spoil(pid, &mut copy);
        }
        Ok(copy)
    }

    /// Returns whether the file's copy `buf` of `pid` is sound and nothing shows it a lost write:
    /// this open has not refused the page as stale, the pool holds it at no later LSN, and its
    /// LSN is not below `mapped`, the page LSN map's entry for it. `image` is its resident copy
    /// under the caller's latch.
    fn trusted(&self, pid: Pid, buf: &[u8], image: Option<&[u8]>, mapped: Option<Lsn>) -> bool {
        let refused = self.meta().damaged.get(&pid).copied();
        let stale = matches!(refused, Some(Damage::Stale { .. }));
        let behind = mapped.is_some_and(|m| page::lsn(buf) < m);
        self.sound(pid, buf)
            && !stale
            && !behind
            && self.lost_write(pid, page::lsn(buf), image).is_none()
    }

    fn copy_log(
        &self,
        fs: &mut Fs<R>,
        run: &mut BackupRun<'_>,
        aside: &str,
        redo: Lsn,
        end: Lsn,
    ) -> Io<Vec<Part>> {
        let (_pin, mut img) = self
            .load_log()
            .map_err(|(what, error)| BackupError::Io { what, error })?;
        let from = img.batch_start(redo).ok_or_else(|| read_failed(redo))?;
        let segment = img.segment;
        let mut parts = Vec::new();
        let mut base = from.0 - from.0 % segment;
        while base < end.0 {
            if !img.bases.contains(&base) {
                return Err(BackupError::Io {
                    what: format!("log segment {}", segment_name(base)),
                    error: IoError::NotFound,
                });
            }
            parts.push(copy_segment(fs, &mut img, run, aside, base, end)?);
            base += segment;
        }
        fs.sync_dir(&inside(aside, PAGES))?;
        fs.sync_dir(&inside(aside, LOG))?;
        Ok(parts)
    }

    /// Returns whether `at` lies on the history of this file: in an epoch the header names, at or
    /// before its end, or in the current one at or before the log's end. A header that has
    /// forgotten epochs takes one older than those it holds, up to the oldest one's end.
    pub fn on_history(&self, at: Position) -> bool {
        let (epoch, ended) = {
            let h = self.header.lock(&self.rt);
            (h.0.epoch, h.0.history.clone())
        };
        if at.epoch == epoch {
            return at.lsn <= self.durable();
        }
        if let Some((_, end)) = ended.iter().find(|(e, _)| *e == at.epoch) {
            return at.lsn <= *end;
        }
        ended.len() == HISTORY_CAPACITY
            && ended
                .first()
                .is_some_and(|(oldest, end)| at.epoch < *oldest && at.lsn <= *end)
    }
}

fn read_failed(at: Lsn) -> BackupError {
    BackupError::Io {
        what: format!("log block at {}", at.0),
        error: IoError::Failed,
    }
}

/// Copy the log of the segment at `base` up to `end`, and return it as a part.
fn copy_segment<R: Runtime>(
    fs: &mut Fs<R>,
    img: &mut LogImage<FileSource<R>>,
    run: &mut BackupRun<'_>,
    aside: &str,
    base: u64,
    end: Lsn,
) -> Io<Part> {
    let name = inside(LOG, &segment_name(base));
    let path = inside(aside, &name);
    let file = fs.create(&path)?;
    let stop = end.0.min(base + img.segment);
    let block = img.block as u64;
    let hash = fs.closing(&name, file, |fs| {
        let mut hash = blake3::Hasher::new();
        let mut at = base;
        while at < stop {
            if !(run.visit)(Pid(0), &mut []) {
                return Err(BackupError::Stopped);
            }
            let upto = (at + IO_CHUNK).min(stop);
            let mut bytes = Vec::with_capacity((upto - at) as usize);
            for b in (at..upto).step_by(block as usize) {
                match img.slot(Lsn(b)) {
                    Slot::Bytes(data) => {
                        if let BlockState::Valid(h, _) = read_block(Lsn(b), &data)
                            && h.file_id != img.file_id
                        {
                            return Err(read_failed(Lsn(b)));
                        }
                        bytes.extend_from_slice(&data);
                    }
                    Slot::Missing | Slot::Unreadable => return Err(read_failed(Lsn(b))),
                }
            }
            hash.update(&bytes);
            fs.write(&name, file, at - base, &bytes)?;
            at = upto;
        }
        fs.sync(&name, file)?;
        Ok(hash)
    })?;
    Ok(Part {
        name,
        len: stop - base,
        hash: hex(hash.finalize().as_bytes()),
    })
}

fn wall_micros(at: std::time::SystemTime) -> i64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
}

/// Returns `path` without the separators that end it, which would put a name beside it inside it.
pub(super) fn final_name(path: &str) -> &str {
    match path.trim_end_matches('/') {
        "" => path,
        name => name,
    }
}

/// Create the directories above `path`. Returns the ones it made, outermost first.
pub(super) fn make_parent<R: Runtime>(
    fs: &mut Fs<R>,
    path: &str,
    taken: impl Fn() -> BackupError,
) -> Io<Vec<String>> {
    let dir = parent(path);
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    fs.make_dirs(dir, taken)
}

/// Answer `taken` when `path` names an entry of any kind.
pub(super) fn name_taken<R: Runtime>(
    fs: &mut Fs<R>,
    path: &str,
    taken: impl FnOnce() -> BackupError,
) -> Io<()> {
    match fs.list(parent(path)) {
        Ok(entries) if !entries.iter().any(|e| e == path) => Ok(()),
        Ok(_)
        | Err(BackupError::Io {
            error: IoError::NotADirectory,
            ..
        }) => Err(taken()),
        Err(e) => Err(e),
    }
}

/// Answer `taken` when `dir` holds an entry of any kind or is not a directory.
pub(super) fn occupied<R: Runtime>(
    fs: &mut Fs<R>,
    dir: &str,
    taken: impl FnOnce() -> BackupError,
) -> Io<()> {
    match fs.list(dir) {
        Ok(entries) if entries.is_empty() => Ok(()),
        Ok(_)
        | Err(BackupError::Io {
            error: IoError::NotADirectory,
            ..
        }) => Err(taken()),
        Err(e) => Err(e),
    }
}

/// Returns the manifest of the backup at `source` and its text, checking its own checksum only.
pub fn read_manifest<R: Runtime>(rt: &R, source: &str) -> Result<(Manifest, String), BackupError> {
    let mut fs = Fs::new(rt.clone());
    manifest_of(&mut fs, source)
}

fn manifest_of<R: Runtime>(
    fs: &mut Fs<R>,
    source: &str,
) -> Result<(Manifest, String), BackupError> {
    let text = fs
        .read_text(&inside(source, MANIFEST))?
        .ok_or_else(|| BackupError::Absent(source.to_owned()))?;
    Ok((Manifest::parse(&text)?, text))
}

/// Check every part `manifest` lists against its length and hash, in the manifest's order.
fn check_parts<R: Runtime>(fs: &mut Fs<R>, source: &str, manifest: &Manifest) -> Io<()> {
    for part in &manifest.parts {
        let damaged = || BackupError::Damaged {
            part: part.name.clone(),
        };
        let path = inside(source, &part.name);
        let (file, len) = match fs.drv.open(&path, false) {
            Ok(opened) => opened,
            Err(IoError::NotFound | IoError::NotADirectory) => return Err(damaged()),
            Err(e) => return Err(io_error(&path)(e)),
        };
        let hash = fs.closing(&path, file, |fs| {
            if len != part.len {
                return Err(damaged());
            }
            let mut hash = blake3::Hasher::new();
            let mut at = 0;
            while at < len {
                let n = IO_CHUNK.min(len - at);
                let bytes = fs.read(&path, file, at, n as usize)?;
                hash.update(&bytes);
                at += n;
            }
            Ok(hash)
        })?;
        if hex(hash.finalize().as_bytes()) != part.hash {
            return Err(damaged());
        }
    }
    Ok(())
}

/// Returns the manifest of the backup at `source` once every part checks.
pub fn check_backup<R: Runtime>(rt: &R, source: &str) -> Result<(Manifest, String), BackupError> {
    let mut fs = Fs::new(rt.clone());
    let (manifest, text) = manifest_of(&mut fs, source)?;
    check_parts(&mut fs, source, &manifest)?;
    Ok((manifest, text))
}

/// Build the database of the backup at `source` aside, check it, and publish it as the new file
/// `path` and its log directory `log_dir`, the file's name last.
///
/// Postconditions:
/// - Until the file's name exists the database does not open, and it is whole once it does.
/// - The file opens in an epoch a random step from 1 to 2^32 above the backup's, and that epoch's
///   end is on its history.
/// - A failed read answers `Io`, and only a log that reads and is not the backup's answers
///   `Damaged`.
/// - On a failure before the file's rename, `path` is as it was and the private copies removed. A
///   directory made above them stays. `log_dir` is as it was, except that an empty directory there
///   is gone once the log's rename replaced it.
/// - A failed rename of the file whose log name cannot be taken back, and a crash between the
///   renames, leave a final `log_dir`, which a retry answers with `Exists`.
pub fn restore_file<R: Runtime>(
    rt: &R,
    source: &str,
    path: &str,
    log_dir: &str,
) -> Result<Manifest, BackupError> {
    let mut fs = Fs::new(rt.clone());
    let (manifest, _) = manifest_of(&mut fs, source)?;
    check_parts(&mut fs, source, &manifest)?;
    let (path, log_dir) = (final_name(path), final_name(log_dir));
    let exists = || BackupError::Exists(path.to_owned());
    name_taken(&mut fs, path, exists)?;
    occupied(&mut fs, log_dir, exists)?;
    make_parent(&mut fs, path, exists)?;
    make_parent(&mut fs, log_dir, exists)?;
    let log_aside = fs.aside_dir(log_dir)?;
    let done = fs.aside_file(path).and_then(|(file_aside, file)| {
        let done = build_restored(
            &mut fs,
            rt,
            source,
            &file_aside,
            file,
            &log_aside,
            &manifest,
        )
        .and_then(|()| publish_restored(&mut fs, &file_aside, path, &log_aside, log_dir));
        if done.is_err() {
            fs.discard(&file_aside).ok();
        }
        done
    });
    if done.is_err() {
        fs.discard(&log_aside).ok();
    }
    done?;
    Ok(manifest)
}

/// Write the file and log aside, check the log, then write the header and close the file.
fn build_restored<R: Runtime>(
    fs: &mut Fs<R>,
    rt: &R,
    source: &str,
    file_aside: &str,
    file: FileId,
    log_aside: &str,
    manifest: &Manifest,
) -> Io<()> {
    fs.closing(file_aside, file, |fs| {
        let page = manifest.header.page_size as usize;
        for part in manifest.parts.iter().filter(|p| is_in(&p.name, PAGES)) {
            write_pages(fs, &inside(source, &part.name), part.len, file, page)?;
        }
        fs.sync(file_aside, file)?;
        for part in manifest.parts.iter().filter(|p| is_in(&p.name, LOG)) {
            let name = part.name.rsplit_once('/').map_or("", |(_, n)| n);
            let to = inside(log_aside, name);
            restore_segment(fs, source, part, &to, manifest.header.segment_size)?;
        }
        fs.sync_dir(log_aside)?;
        if !restored_log_ends(fs, rt, manifest, log_aside)? {
            return Err(BackupError::Damaged {
                part: LOG.to_owned(),
            });
        }
        write_header(fs, rt, file_aside, file, manifest)
    })
}

/// Rename the log directory and then the file, the commit point. A failure before the file's name
/// exists takes the log directory's name back.
pub(super) fn publish_restored<R: Runtime>(
    fs: &mut Fs<R>,
    file_aside: &str,
    path: &str,
    log_aside: &str,
    log_dir: &str,
) -> Io<()> {
    let exists = || BackupError::Exists(path.to_owned());
    fs.sync_dir(parent(path))?;
    fs.sync_dir(parent(log_dir))?;
    fs.rename(log_aside, log_dir, exists)?;
    let file = fs
        .sync_dir(parent(log_dir))
        .and_then(|()| fs.rename(file_aside, path, exists));
    if file.is_err() {
        fs.rename(log_dir, log_aside, exists).ok();
    }
    file?;
    fs.sync_dir(parent(path))
}

/// Returns whether the log in `log_dir` ends at the end of `manifest`.
fn restored_log_ends<R: Runtime>(
    fs: &mut Fs<R>,
    rt: &R,
    manifest: &Manifest,
    log_dir: &str,
) -> Io<bool> {
    let mut files = BTreeMap::new();
    let mut out = open_segments(fs, log_dir, &mut files).and_then(|()| {
        let (block, segment) = (
            manifest.header.log_block as usize,
            manifest.header.segment_size,
        );
        let file_id = manifest.header.file_id;
        let mut img = FileSource::image(rt.clone(), block, segment, file_id, files.clone());
        log_ends(&mut img, manifest)
    });
    for file in files.into_values() {
        let closed = fs.close(log_dir, file);
        out = out.and_then(|ends| closed.map(|()| ends));
    }
    out
}

fn open_segments<R: Runtime>(
    fs: &mut Fs<R>,
    log_dir: &str,
    files: &mut BTreeMap<u64, FileId>,
) -> Io<()> {
    for path in fs.list(log_dir)? {
        if let Some(base) = parse_segment(&path) {
            let (file, _) = fs.open(&path, false)?;
            files.insert(base, file);
        }
    }
    Ok(())
}

/// Returns whether the log is the one recovery will accept: read from the backup's start without a
/// gap to its end, holding its checkpoint.
fn log_ends<S: LogSource>(img: &mut LogImage<S>, manifest: &Manifest) -> Io<bool> {
    let start = manifest.start.lsn;
    let first = Lsn(start.0 - start.0 % img.block as u64);
    if img.slot(first) == Slot::Unreadable {
        return Err(read_failed(first));
    }
    let Some(from) = img.batch_start(start) else {
        return Ok(false);
    };
    let block = img.block;
    let mut names_checkpoint = false;
    let scanned = scan(img, from, Mode::Recovery, |batch, _| {
        names_checkpoint |= holds_checkpoint(batch, block, manifest);
    });
    let unreadable = scanned
        .findings
        .iter()
        .find(|f| f.problem == LogProblem::Unreadable);
    if let Some(finding) = unreadable {
        return Err(read_failed(finding.lsn));
    }
    Ok(names_checkpoint
        && scanned.fatal.is_none()
        && scanned.first == Some(from)
        && scanned.end == manifest.end.lsn)
}

/// Returns whether `batch` holds the header's checkpoint, a checkpoint record whose redo point is
/// the backup's start.
fn holds_checkpoint(batch: &Batch, block: usize, manifest: &Manifest) -> bool {
    let at = manifest.header.checkpoint;
    batch.records(block).is_ok_and(|records| {
        records.into_iter().any(|(lsn, record)| {
            lsn == at
                && matches!(record, Record::Checkpoint { redo, .. } if redo == manifest.start.lsn)
        })
    })
}

fn is_in(name: &str, dir: &str) -> bool {
    name.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn write_pages<R: Runtime>(
    fs: &mut Fs<R>,
    from: &str,
    len: u64,
    to: FileId,
    page: usize,
) -> Io<()> {
    let (file, _) = fs.open(from, false)?;
    fs.closing(from, file, |fs| {
        let mut at = 0;
        while at < len {
            let head = fs.read(from, file, at, REGION)?;
            let word = |i: usize| {
                u64::from_le_bytes(head[i * 8..i * 8 + 8].try_into().expect("eight bytes"))
            };
            let count = usize::try_from(word(0)).unwrap_or(0);
            let pages = fs.read(from, file, at + REGION as u64, count * page)?;
            for i in 0..count {
                let pid = word(i + 1);
                let bytes = &pages[i * page..(i + 1) * page];
                fs.write(from, to, pid * page as u64, bytes)?;
            }
            at += (REGION + count * page) as u64;
        }
        Ok(())
    })
}

fn restore_segment<R: Runtime>(
    fs: &mut Fs<R>,
    source: &str,
    part: &Part,
    to: &str,
    segment: u64,
) -> Io<()> {
    let from = inside(source, &part.name);
    let (input, _) = fs.open(&from, false)?;
    fs.closing(&from, input, |fs| {
        let file = fs.create(to)?;
        fs.closing(to, file, |fs| {
            let mut at = 0;
            while at < part.len {
                let n = IO_CHUNK.min(part.len - at);
                let bytes = fs.read(&from, input, at, n as usize)?;
                fs.write(to, file, at, &bytes)?;
                at += n;
            }
            fs.run(to, Op::Allocate { file, len: segment })?;
            fs.sync(to, file)
        })
    })
}

fn write_header<R: Runtime>(
    fs: &mut Fs<R>,
    rt: &R,
    path: &str,
    file: FileId,
    m: &Manifest,
) -> Io<()> {
    let mut header = m.header.clone();
    header.history.push((m.end.epoch, m.end.lsn));
    let excess = header.history.len().saturating_sub(HISTORY_CAPACITY);
    header.history.drain(..excess);
    header.epoch = m.end.epoch + rt.next_u64() % EPOCH_STEP + 1;
    header.log_start = m.end.lsn;
    for (seq, copy) in [(1, 1u64), (2, 0)] {
        header.seq = seq;
        let offset = copy * u64::from(header.page_size);
        fs.write(path, file, offset, &header.encode(copy))?;
        fs.sync(path, file)?;
    }
    Ok(())
}

/// What a test restore of a backup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proved {
    Proven,
    Failed(Vec<String>),
    /// Nothing was found wrong and nothing shown right, for want of a host function.
    Unproven {
        function: String,
    },
}

/// Record `proved` of the manifest named `hash` in the proof file beside it, whole. The last writer
/// wins.
pub fn write_proof<R: Runtime>(rt: &R, source: &str, hash: &str, proved: &Proved) -> Io<()> {
    let mut fs = Fs::new(rt.clone());
    let mut body = format!("{PROOF_LINE}\nmanifest {hash}\n");
    match proved {
        Proved::Proven => body.push_str("result proven\n"),
        Proved::Unproven { function } => {
            writeln!(body, "result unproven {function}").expect("a string takes any text");
        }
        Proved::Failed(findings) => {
            body.push_str("result failed\n");
            for f in findings {
                writeln!(body, "finding {}", f.replace('\n', " "))
                    .expect("a string takes any text");
            }
        }
    }
    let proof = inside(source, PROOF);
    let (aside, file) = fs.aside_file(&proof)?;
    let op = Op::Rename {
        from: aside.clone(),
        to: proof.clone(),
        replace: true,
    };
    let done = fs.write_text(&aside, file, &sealed(body)).and_then(|()| {
        fs.run(&proof, op)?;
        fs.sync_dir(source)
    });
    if done.is_err() {
        fs.discard(&aside).ok();
    }
    done
}

/// Remove the proof file of the backup at `source`, which a damaged part has refuted.
pub fn withdraw_proof<R: Runtime>(rt: &R, source: &str) -> Io<()> {
    let mut fs = Fs::new(rt.clone());
    fs.remove(&inside(source, PROOF))?;
    fs.sync_dir(source)
}

/// Returns what the proof file of the manifest named `hash` records, `None` when no whole proof
/// file names it.
pub fn read_proof<R: Runtime>(rt: &R, source: &str, hash: &str) -> Io<Option<Proved>> {
    let mut fs = Fs::new(rt.clone());
    let Some(text) = fs.read_text(&inside(source, PROOF))? else {
        return Ok(None);
    };
    Ok(parse_proof(&text, hash))
}

fn parse_proof(text: &str, hash: &str) -> Option<Proved> {
    let lines = unsealed(text)?;
    let mut lines = lines.lines();
    if lines.next() != Some(PROOF_LINE) || lines.next()?.strip_prefix("manifest ")? != hash {
        return None;
    }
    let result = lines.next()?.strip_prefix("result ")?;
    if result == "proven" {
        return Some(Proved::Proven);
    }
    if let Some(function) = result.strip_prefix("unproven ") {
        return Some(Proved::Unproven {
            function: function.to_owned(),
        });
    }
    let findings = lines
        .filter_map(|l| l.strip_prefix("finding "))
        .map(str::to_owned);
    Some(Proved::Failed(findings.collect()))
}

/// Create a private scratch directory in the backup at `source`, and return its path.
pub fn make_scratch<R: Runtime>(rt: &R, source: &str) -> Io<String> {
    Fs::new(rt.clone()).aside_dir(&inside(source, SCRATCH))
}

/// Returns the path of the database a test restore rebuilds in the scratch directory `scratch`.
pub fn scratch_file(scratch: &str) -> String {
    inside(scratch, "restored.adt")
}

/// Remove the scratch directory `scratch` and the database in it.
pub fn remove_scratch<R: Runtime>(rt: &R, scratch: &str) -> Io<()> {
    Fs::new(rt.clone()).discard(scratch)
}

impl<S: LogSource> LogImage<S> {
    /// Returns the first batch beginning in the first segment. A batch may have started in the
    /// segment before it, which the image lacks.
    pub fn first_batch(&mut self) -> Option<Lsn> {
        let base = self.first()?;
        if let Slot::Bytes(b) = self.slot(base)
            && let BlockState::Valid(h, _) = read_block(base, &b)
            && h.index > 0
        {
            let left = u64::from(h.count - h.index);
            return Some(Lsn(base.0 + left * self.block as u64));
        }
        Some(base)
    }
}
