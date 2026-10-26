use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;

use super::files::{Files, Outcome};
use crate::runtime::{AlignedBuf, Done, FileId, IoError, Op};

/// The faults an [`OsRuntime`](super::OsRuntime) injects beneath the engine's files.
///
/// Each `_p` is the chance of its fault per op that can suffer it, drawn from a generator seeded
/// by `seed`. The I/O threads draw in the order the host runs them, so a seed does not replay a
/// run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Faults {
    pub seed: u64,
    /// Chance that a write lost at a crash, and larger than the atomic unit, is lost torn: any
    /// subset of its atomic units reaches the device.
    pub torn_p: f64,
    /// Chance that a read flips one stored bit in its range first.
    pub corrupt_p: f64,
    /// Chance that a write lands at another offset of the same file.
    pub misdirect_p: f64,
    /// Chance that a write reports success and never reaches the device.
    pub lost_write_p: f64,
    /// Chance that a read makes one sector in its range unreadable until it is rewritten.
    pub latent_p: f64,
    /// Chance that a write, an allocation or a force fails.
    pub fail_p: f64,
    /// Lose any subset of unforced writes, and of a failed force's writes, and any suffix of the
    /// unforced creations and removals, at a crash.
    pub lose_unsynced: bool,
    /// The bytes a latent error makes unreadable, a multiple of the device's logical block.
    pub sector: u64,
    /// The bytes the device writes atomically, a multiple of `sector`.
    pub atomic_unit: u64,
}

impl Default for Faults {
    fn default() -> Self {
        Faults {
            seed: 0,
            torn_p: 0.0,
            corrupt_p: 0.0,
            misdirect_p: 0.0,
            lost_write_p: 0.0,
            latent_p: 0.0,
            fail_p: 0.0,
            lose_unsynced: true,
            sector: 4096,
            atomic_unit: 4096,
        }
    }
}

/// The state of the injected faults, behind one lock that every op takes.
#[derive(Debug)]
pub(crate) struct Injector {
    faults: Faults,
    rng: u64,
    crashed: bool,
    /// The path of each file open in this process and not removed.
    paths: HashMap<FileId, String>,
    /// Each file's writes since its last force.
    pending: BTreeMap<String, Pending>,
    /// Sectors unreadable until rewritten, by path and sector index.
    bad: BTreeSet<(String, u64)>,
    /// The creations and removals no completed `SyncDir` covers, in completion order.
    entries: Vec<Entry>,
}

#[derive(Debug, Default)]
struct Pending {
    /// The contents of each sector written since the last force, as the force left it.
    base: BTreeMap<u64, Vec<u8>>,
    writes: Vec<(u64, Vec<u8>)>,
}

#[derive(Debug)]
struct Entry {
    parent: String,
    undo: Undo,
}

#[derive(Debug)]
enum Undo {
    Create { path: String, dir: bool },
    Restore { path: String, bytes: Vec<u8> },
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn buffer(op: Op) -> Option<AlignedBuf> {
    match op {
        Op::Read { buf, .. } | Op::Write { buf, .. } => Some(buf),
        _ => None,
    }
}

fn check(faults: &Faults) {
    assert!(
        faults.sector > 0 && faults.atomic_unit.is_multiple_of(faults.sector),
        "the atomic unit is a whole number of sectors"
    );
}

fn failed<T>(_: io::Error) -> Result<T, IoError> {
    Err(IoError::Failed)
}

impl Injector {
    pub(crate) fn new(faults: Faults) -> Injector {
        check(&faults);
        Injector {
            faults,
            rng: faults.seed,
            crashed: false,
            paths: HashMap::new(),
            pending: BTreeMap::new(),
            bad: BTreeSet::new(),
            entries: Vec::new(),
        }
    }

    /// Inject `faults` from now on, keeping what earlier faults left and the generator's state.
    pub(crate) fn set(&mut self, faults: Faults) {
        check(&faults);
        self.faults = faults;
    }

    fn next(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn draw(&mut self, p: f64) -> bool {
        #[expect(
            clippy::cast_precision_loss,
            reason = "53 bits fit an f64's mantissa exactly"
        )]
        let unit = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        unit < p
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Run `op` on `files` with the faults injected.
    pub(crate) fn run(&mut self, files: &Files, op: Op) -> Outcome {
        if self.crashed {
            return (buffer(op), Err(IoError::Failed));
        }
        match op {
            Op::Open { path, create } => self.open(files, path, create),
            Op::Read { file, offset, buf } => self.read(files, file, offset, buf),
            Op::Write { file, offset, buf } => self.write(files, file, offset, buf),
            Op::Allocate { .. } if self.draw(self.faults.fail_p) => (None, Err(IoError::Failed)),
            Op::Sync { file } => self.sync(files, file),
            Op::SyncDir { dir } => {
                if self.draw(self.faults.fail_p) {
                    return (None, Err(IoError::Failed));
                }
                let out = files.perform(Op::SyncDir { dir: dir.clone() });
                if out.1.is_ok() {
                    self.entries.retain(|e| e.parent != dir);
                }
                out
            }
            Op::Remove { path } => self.remove(files, path),
            Op::CreateDir { dir } => {
                let existed = files.path(&dir).is_dir();
                let out = files.perform(Op::CreateDir { dir: dir.clone() });
                if out.1.is_ok() && !existed {
                    self.created(dir, true);
                }
                out
            }
            Op::Close { file } => {
                self.paths.remove(&file);
                files.perform(Op::Close { file })
            }
            op => files.perform(op),
        }
    }

    fn created(&mut self, path: String, dir: bool) {
        if self.faults.lose_unsynced {
            self.entries.push(Entry {
                parent: parent(&path).to_owned(),
                undo: Undo::Create { path, dir },
            });
        }
    }

    fn open(&mut self, files: &Files, path: String, create: bool) -> Outcome {
        let existed = files.path(&path).exists();
        let out = files.perform(Op::Open {
            path: path.clone(),
            create,
        });
        if let Ok(Done::Opened { file, .. }) = out.1 {
            self.paths.insert(file, path.clone());
            if !existed {
                self.created(path, false);
            }
        }
        out
    }

    fn remove(&mut self, files: &Files, path: String) -> Outcome {
        let bytes = if self.faults.lose_unsynced {
            fs::read(files.path(&path)).ok()
        } else {
            None
        };
        let out = files.perform(Op::Remove { path: path.clone() });
        if out.1.is_ok() {
            self.pending.remove(&path);
            self.paths.retain(|_, p| *p != path);
            self.bad.retain(|(p, _)| *p != path);
            if let Some(bytes) = bytes {
                self.entries.push(Entry {
                    parent: parent(&path).to_owned(),
                    undo: Undo::Restore { path, bytes },
                });
            }
        }
        out
    }

    fn sectors(&self, offset: u64, len: u64) -> std::ops::Range<u64> {
        let s = self.faults.sector;
        offset / s..(offset + len).div_ceil(s)
    }

    fn read(&mut self, files: &Files, file: FileId, offset: u64, buf: AlignedBuf) -> Outcome {
        let Some(path) = self.paths.get(&file).cloned() else {
            return files.perform(Op::Read { file, offset, buf });
        };
        let len = buf.len() as u64;
        let mut range = self.sectors(offset, len);
        if range.any(|s| self.bad.contains(&(path.clone(), s))) {
            return (Some(buf), Err(IoError::Failed));
        }
        let (mut buf, n) = match files.perform(Op::Read { file, offset, buf }) {
            (Some(buf), Ok(Done::Read(n))) => (buf, n),
            out => return out,
        };
        let n = n as u64;
        if n > 0 && self.draw(self.faults.latent_p) {
            let sectors = self.sectors(offset, n);
            let s = sectors.start + self.below(sectors.end - sectors.start);
            self.bad.insert((path, s));
            return (Some(buf), Err(IoError::Failed));
        }
        if n > 0 && self.draw(self.faults.corrupt_p) {
            let at = self.below(n);
            let bit = 1u8 << self.below(8);
            buf[at as usize] ^= bit;
            let stored = AlignedBuf::from_slice(&buf[..n as usize]);
            if files.file(file).write_all_at(&stored, offset).is_err() {
                return (Some(buf), Err(IoError::Failed));
            }
            let byte = offset + at;
            let sector = self.faults.sector;
            if let Some(base) = self
                .pending
                .get_mut(&path)
                .and_then(|p| p.base.get_mut(&(byte / sector)))
            {
                base[(byte % sector) as usize] ^= bit;
            }
        }
        (Some(buf), Ok(Done::Read(n as usize)))
    }

    fn write(&mut self, files: &Files, file: FileId, offset: u64, buf: AlignedBuf) -> Outcome {
        let Some(path) = self.paths.get(&file).cloned() else {
            return files.perform(Op::Write { file, offset, buf });
        };
        if self.draw(self.faults.lost_write_p) {
            return (Some(buf), Ok(Done::Written));
        }
        let fd = files.file(file);
        let len = buf.len() as u64;
        if self.draw(self.faults.fail_p) {
            let mut at = 0;
            while at < len {
                let n = self.faults.sector.min(len - at);
                if self.draw(0.5) {
                    let part = AlignedBuf::from_slice(&buf[at as usize..(at + n) as usize]);
                    if self.land(&fd, &path, offset + at, &part).is_err() {
                        break;
                    }
                }
                at += n;
            }
            return (Some(buf), Err(IoError::Failed));
        }
        let mut target = offset;
        if self.draw(self.faults.misdirect_p) {
            let end = fd.metadata().map_or(0, |m| m.len()).max(offset + len);
            let slots = end / len;
            if slots > 1 {
                let slot = self.below(slots - 1);
                target = if slot >= offset / len { slot + 1 } else { slot } * len;
            }
        }
        let result = self.land(&fd, &path, target, &buf).or_else(failed);
        (Some(buf), result.map(|()| Done::Written))
    }

    /// Write `bytes` at `offset` of the file at `path`, remembering what it replaces.
    fn land(&mut self, fd: &File, path: &str, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let range = self.sectors(offset, bytes.len() as u64);
        if self.faults.lose_unsynced {
            let sector = self.faults.sector;
            let pending = self.pending.entry(path.to_owned()).or_default();
            for s in range.clone() {
                if pending.base.contains_key(&s) {
                    continue;
                }
                let mut old = AlignedBuf::zeroed(sector as usize);
                fd.read_at(&mut old, s * sector)?;
                pending.base.insert(s, old.to_vec());
            }
            pending.writes.push((offset, bytes.to_vec()));
        }
        fd.write_all_at(bytes, offset)?;
        for s in range {
            self.bad.remove(&(path.to_owned(), s));
        }
        Ok(())
    }

    fn sync(&mut self, files: &Files, file: FileId) -> Outcome {
        let path = self.paths.get(&file).cloned();
        if self.draw(self.faults.fail_p) {
            if let Some(path) = path
                && let Some(pending) = self.pending.remove(&path)
            {
                self.lose(files, &path, pending);
            }
            return (None, Err(IoError::Failed));
        }
        let out = files.perform(Op::Sync { file });
        if out.1.is_ok()
            && let Some(path) = path
        {
            self.pending.remove(&path);
        }
        out
    }

    /// Put back on the device any subset of `pending`'s writes, each whole or torn, over the
    /// contents the last force left.
    fn lose(&mut self, files: &Files, path: &str, pending: Pending) {
        let unit = self.faults.atomic_unit;
        let mut kept = Vec::new();
        for (offset, bytes) in pending.writes {
            let len = bytes.len() as u64;
            if len > unit && self.draw(self.faults.torn_p) {
                let mut at = 0;
                while at < len {
                    let n = unit.min(len - at);
                    if self.draw(0.5) {
                        kept.push((offset + at, bytes[at as usize..(at + n) as usize].to_vec()));
                    }
                    at += n;
                }
            } else if self.draw(0.5) {
                kept.push((offset, bytes));
            }
        }
        let sector = self.faults.sector;
        let full = files.path(path);
        let fd = OpenOptions::new()
            .write(true)
            .open(&full)
            .unwrap_or_else(|e| panic!("reopen {full:?} to lose its writes: {e}"));
        let end = fd.metadata().map_or(0, |m| m.len());
        for (s, mut contents) in pending.base {
            let start = s * sector;
            if start >= end {
                continue;
            }
            for (offset, bytes) in &kept {
                let lo = start.max(*offset);
                let hi = (start + sector).min(offset + bytes.len() as u64);
                if lo < hi {
                    contents[(lo - start) as usize..(hi - start) as usize]
                        .copy_from_slice(&bytes[(lo - offset) as usize..(hi - offset) as usize]);
                }
            }
            let n = sector.min(end - start) as usize;
            fd.write_all_at(&contents[..n], start)
                .unwrap_or_else(|e| panic!("lose the writes to {full:?}: {e}"));
        }
        fd.sync_all()
            .unwrap_or_else(|e| panic!("force the lost writes to {full:?}: {e}"));
    }

    /// Stop the device as a power loss would, losing what the faults allow.
    pub(crate) fn crash(&mut self, files: &Files) {
        self.crashed = true;
        for (path, pending) in std::mem::take(&mut self.pending) {
            self.lose(files, &path, pending);
        }
        let keep = self.below(self.entries.len() as u64 + 1) as usize;
        for entry in self.entries.drain(keep..).rev() {
            let r = match entry.undo {
                Undo::Create { path, dir: true } => fs::remove_dir_all(files.path(&path)),
                Undo::Create { path, dir: false } => fs::remove_file(files.path(&path)),
                Undo::Restore { path, bytes } => fs::write(files.path(&path), bytes),
            };
            match r {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    panic!("undo an unforced creation or removal: {e}")
                }
                _ => {}
            }
        }
    }
}
