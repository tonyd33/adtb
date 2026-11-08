use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::ops::Range;
use std::os::unix::fs::FileExt;

use super::files::{Files, Outcome, error};
use crate::runtime::{
    AlignedBuf, AllocateFate, Done, Draw, Faults, FileId, ForceFate, IoError, Landing, Op,
    ReadFate, WriteFate, decide_allocate, decide_force, decide_read, decide_unforced, decide_write,
};

/// The state of the injected faults, behind one lock taken to admit and to complete each op.
#[derive(Debug)]
pub(crate) struct Injector {
    faults: Faults,
    rng: SplitMix,
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
    Create {
        path: String,
        dir: bool,
    },
    Restore {
        path: String,
        bytes: Vec<u8>,
    },
    RemovedDir {
        path: String,
    },
    Renamed {
        from: String,
        to: String,
        over: Over,
    },
}

/// What a rename's destination held.
#[derive(Debug)]
enum Over {
    Nothing,
    EmptyDir,
    File(Vec<u8>),
}

/// The unforced writes a crash dropped whole and tore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loss {
    /// The writes no part of which reached the device.
    pub lost: usize,
    /// The writes some but not all of whose atomic units reached the device.
    pub torn: usize,
}

/// What the injector does once the device completes an op it admitted.
#[derive(Debug)]
pub(crate) struct After(Hook);

#[derive(Debug)]
enum Hook {
    Nothing,
    Opened {
        path: String,
        existed: bool,
    },
    Read {
        path: String,
        file: FileId,
        offset: u64,
    },
    Written {
        path: String,
        sectors: Range<u64>,
    },
    /// The writes the force covers, returned to pending if the device fails it.
    Forced(Option<(String, Pending)>),
    SyncedDir {
        dir: String,
    },
    Removed {
        path: String,
        bytes: Option<Vec<u8>>,
    },
    RemovedDir {
        path: String,
    },
    Renamed {
        from: String,
        to: String,
        over: Over,
    },
    CreatedDir {
        dir: String,
        existed: bool,
    },
}

/// Returns `path` with its leading `from` changed to `to`, when `path` is `from` or lies under it.
fn moved(path: &str, from: &str, to: &str) -> Option<String> {
    let rest = path.strip_prefix(from)?;
    (rest.is_empty() || rest.starts_with('/')).then(|| format!("{to}{rest}"))
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
        faults.sector > 0 && faults.sector.is_multiple_of(AlignedBuf::ALIGN as u64),
        "a sector is a whole number of I/O buffer alignments, so a sector-sized direct read or \
         write is block aligned"
    );
    assert!(
        faults.atomic_unit.is_multiple_of(faults.sector),
        "the atomic unit is a whole number of sectors"
    );
}

/// SplitMix64, seeded by the faults' `seed`.
#[derive(Debug)]
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

impl Draw for SplitMix {
    fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

impl Injector {
    pub(crate) fn new(faults: Faults) -> Injector {
        check(&faults);
        Injector {
            faults,
            rng: SplitMix(faults.seed),
            crashed: false,
            paths: HashMap::new(),
            pending: BTreeMap::new(),
            bad: BTreeSet::new(),
            entries: Vec::new(),
        }
    }

    /// Inject `faults` from now on, keeping what earlier faults left and the generator's state.
    pub(crate) fn set(&mut self, faults: Faults) {
        let now = &self.faults;
        assert!(
            (faults.sector, faults.atomic_unit) == (now.sector, now.atomic_unit),
            "the device's geometry is fixed for the runtime's life: sector {} and atomic unit {}, \
             not {} and {}",
            now.sector,
            now.atomic_unit,
            faults.sector,
            faults.atomic_unit
        );
        self.faults = faults;
    }

    /// Run `op` on `files` with the faults injected.
    pub(crate) fn run(&mut self, files: &Files, op: Op) -> Outcome {
        match self.admit(files, op) {
            Ok((op, after)) => {
                let out = files.perform(op);
                self.complete(files, after, out)
            }
            Err(out) => out,
        }
    }

    /// Returns `op` as the device is to run it and what follows its completion, or its outcome
    /// when a fault completes it without the device.
    pub(crate) fn admit(&mut self, files: &Files, op: Op) -> Result<(Op, After), Outcome> {
        if self.crashed {
            return Err((buffer(op), Err(IoError::Failed)));
        }
        let hook = match op {
            Op::Open { ref path, .. } => Hook::Opened {
                path: path.clone(),
                existed: files.path(path).exists(),
            },
            Op::CreateNew { ref path } => Hook::Opened {
                path: path.clone(),
                existed: false,
            },
            Op::Read { file, offset, buf } => {
                let hook = match self.paths.get(&file) {
                    Some(path) => Hook::Read {
                        path: path.clone(),
                        file,
                        offset,
                    },
                    None => Hook::Nothing,
                };
                return Ok((Op::Read { file, offset, buf }, After(hook)));
            }
            Op::Write { file, offset, buf } => return self.admit_write(files, file, offset, buf),
            Op::Allocate { file, len } => {
                let file_len = match files.file(file).metadata() {
                    Ok(m) => m.len(),
                    Err(e) => return Err((None, Err(error(&e)))),
                };
                match decide_allocate(&mut self.rng, &self.faults, len, file_len) {
                    AllocateFate::Fail => return Err((None, Err(IoError::Failed))),
                    AllocateFate::Full => return Err((None, Err(IoError::Full))),
                    AllocateFate::Run => Hook::Nothing,
                }
            }
            Op::Sync { file } => return self.admit_sync(files, file),
            Op::SyncDir { .. } if decide_force(&mut self.rng, &self.faults) == ForceFate::Fail => {
                return Err((None, Err(IoError::Failed)));
            }
            Op::SyncDir { ref dir } => Hook::SyncedDir { dir: dir.clone() },
            Op::Remove { ref path } => Hook::Removed {
                path: path.clone(),
                bytes: if self.faults.lose_unsynced {
                    fs::read(files.path(path)).ok()
                } else {
                    None
                },
            },
            Op::RemoveDir { ref path } => Hook::RemovedDir { path: path.clone() },
            Op::Rename {
                ref from, ref to, ..
            } => {
                let target = files.path(to);
                let over = if target.is_dir() {
                    Over::EmptyDir
                } else {
                    fs::read(&target).map_or(Over::Nothing, Over::File)
                };
                Hook::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                    over,
                }
            }
            Op::CreateDir { ref dir } => Hook::CreatedDir {
                dir: dir.clone(),
                existed: files.path(dir).is_dir(),
            },
            Op::Close { file } => {
                self.paths.remove(&file);
                Hook::Nothing
            }
            _ => Hook::Nothing,
        };
        Ok((op, After(hook)))
    }

    /// Returns the outcome of an op [`Injector::admit`] passed to the device, given what the
    /// device answered.
    pub(crate) fn complete(&mut self, files: &Files, after: After, out: Outcome) -> Outcome {
        if self.crashed {
            return (out.0, Err(IoError::Failed));
        }
        match after.0 {
            Hook::Read { path, file, offset } => return self.read(files, path, file, offset, out),
            Hook::Forced(Some((path, forced))) if out.1.is_err() => self.unforced(path, forced),
            _ if out.1.is_err() => {}
            Hook::Opened { path, existed } => {
                if let Ok(Done::Opened { file, .. }) = out.1 {
                    self.paths.insert(file, path.clone());
                    if !existed {
                        self.created(path, false);
                    }
                }
            }
            Hook::Written { path, sectors } => {
                for s in sectors {
                    self.bad.remove(&(path.clone(), s));
                }
            }
            Hook::SyncedDir { dir } => self.entries.retain(|e| e.parent != dir),
            Hook::Removed { path, bytes } => self.removed(path, bytes),
            Hook::RemovedDir { path } => {
                if self.faults.lose_unsynced {
                    self.entries.push(Entry {
                        parent: parent(&path).to_owned(),
                        undo: Undo::RemovedDir { path },
                    });
                }
            }
            Hook::Renamed { from, to, over } => self.renamed(from, to, over),
            Hook::CreatedDir { dir, existed } => {
                if !existed {
                    self.created(dir, true);
                }
            }
            Hook::Forced(_) | Hook::Nothing => {}
        }
        out
    }

    fn created(&mut self, path: String, dir: bool) {
        if self.faults.lose_unsynced {
            self.entries.push(Entry {
                parent: parent(&path).to_owned(),
                undo: Undo::Create { path, dir },
            });
        }
    }

    fn removed(&mut self, path: String, bytes: Option<Vec<u8>>) {
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

    fn renamed(&mut self, from: String, to: String, over: Over) {
        for path in self.paths.values_mut() {
            if let Some(now) = moved(path, &from, &to) {
                *path = now;
            }
        }
        let keys: Vec<String> = self.pending.keys().cloned().collect();
        for key in keys {
            if let Some(now) = moved(&key, &from, &to)
                && let Some(pending) = self.pending.remove(&key)
            {
                self.pending.insert(now, pending);
            }
        }
        self.bad = std::mem::take(&mut self.bad)
            .into_iter()
            .map(|(p, s)| (moved(&p, &from, &to).unwrap_or(p), s))
            .collect();
        if self.faults.lose_unsynced {
            self.entries.push(Entry {
                parent: parent(&to).to_owned(),
                undo: Undo::Renamed { from, to, over },
            });
        }
    }

    fn sectors(&self, offset: u64, len: u64) -> Range<u64> {
        let s = self.faults.sector;
        offset / s..(offset + len).div_ceil(s)
    }

    fn read(
        &mut self,
        files: &Files,
        path: String,
        file: FileId,
        offset: u64,
        out: Outcome,
    ) -> Outcome {
        let (mut buf, n) = match out {
            (Some(buf), Ok(Done::Read(n))) => (buf, n),
            out => return out,
        };
        let n = n as u64;
        let unreadable = self
            .sectors(offset, n)
            .any(|s| self.bad.contains(&(path.clone(), s)));
        match decide_read(&mut self.rng, &self.faults, offset, n, unreadable) {
            ReadFate::Latent { sector } => {
                self.bad.insert((path, sector));
                return (Some(buf), Err(IoError::Failed));
            }
            ReadFate::Unreadable => return (Some(buf), Err(IoError::Failed)),
            ReadFate::Corrupt { byte, bit } => {
                buf[(byte - offset) as usize] ^= bit;
                let stored = AlignedBuf::from_slice(&buf[..n as usize]);
                let Ok(()) = files.file(file).write_all_at(&stored, offset) else {
                    return (Some(buf), Err(IoError::Failed));
                };
                let sector = self.faults.sector;
                if let Some(base) = self
                    .pending
                    .get_mut(&path)
                    .and_then(|p| p.base.get_mut(&(byte / sector)))
                {
                    base[(byte % sector) as usize] ^= bit;
                }
            }
            ReadFate::Clean => {}
        }
        (Some(buf), Ok(Done::Read(n as usize)))
    }

    fn admit_write(
        &mut self,
        files: &Files,
        file: FileId,
        offset: u64,
        buf: AlignedBuf,
    ) -> Result<(Op, After), Outcome> {
        let Some(path) = self.paths.get(&file).cloned() else {
            return Ok((Op::Write { file, offset, buf }, After(Hook::Nothing)));
        };
        let fd = files.file(file);
        let len = buf.len() as u64;
        let file_len = match fd.metadata() {
            Ok(m) => m.len(),
            Err(e) => return Err((Some(buf), Err(error(&e)))),
        };
        let target = match decide_write(&mut self.rng, &self.faults, offset, len, file_len) {
            WriteFate::Full => return Err((Some(buf), Err(IoError::Full))),
            WriteFate::Fail { landed } => {
                for part in landed {
                    let at = (part.start - offset) as usize..(part.end - offset) as usize;
                    let part_buf = AlignedBuf::from_slice(&buf[at]);
                    let Ok(()) = self.land(&fd, &path, part.start, &part_buf) else {
                        break;
                    };
                }
                return Err((Some(buf), Err(IoError::Failed)));
            }
            WriteFate::Lost => return Err((Some(buf), Ok(Done::Written))),
            WriteFate::At(target) => target,
        };
        if self.record(&fd, &path, target, &buf).is_err() {
            return Err((Some(buf), Err(IoError::Failed)));
        }
        let sectors = self.sectors(target, len);
        let op = Op::Write {
            file,
            offset: target,
            buf,
        };
        Ok((op, After(Hook::Written { path, sectors })))
    }

    /// Write `bytes` at `offset` of the file at `path`, remembering what it replaces.
    fn land(&mut self, fd: &File, path: &str, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.record(fd, path, offset, bytes)?;
        fd.write_all_at(bytes, offset)?;
        for s in self.sectors(offset, bytes.len() as u64) {
            self.bad.remove(&(path.to_owned(), s));
        }
        Ok(())
    }

    /// Remember a write of `bytes` at `offset` of the file at `path` and what it replaces.
    fn record(&mut self, fd: &File, path: &str, offset: u64, bytes: &[u8]) -> io::Result<()> {
        if !self.faults.lose_unsynced {
            return Ok(());
        }
        let sector = self.faults.sector;
        let range = self.sectors(offset, bytes.len() as u64);
        let pending = self.pending.entry(path.to_owned()).or_default();
        for s in range {
            if pending.base.contains_key(&s) {
                continue;
            }
            let mut old = AlignedBuf::zeroed(sector as usize);
            fd.read_at(&mut old, s * sector)?;
            pending.base.insert(s, old.to_vec());
        }
        pending.writes.push((offset, bytes.to_vec()));
        Ok(())
    }

    fn admit_sync(&mut self, files: &Files, file: FileId) -> Result<(Op, After), Outcome> {
        let path = self.paths.get(&file).cloned();
        if decide_force(&mut self.rng, &self.faults) == ForceFate::Fail {
            if let Some(path) = path
                && let Some(pending) = self.pending.remove(&path)
            {
                self.lose(files, &path, pending);
            }
            return Err((None, Err(IoError::Failed)));
        }
        let forced = path.and_then(|p| self.pending.remove(&p).map(|f| (p, f)));
        Ok((Op::Sync { file }, After(Hook::Forced(forced))))
    }

    /// Return the writes a force that failed on the device took, ahead of any written since.
    fn unforced(&mut self, path: String, mut forced: Pending) {
        if let Some(later) = self.pending.remove(&path) {
            for (s, contents) in later.base {
                forced.base.entry(s).or_insert(contents);
            }
            forced.writes.extend(later.writes);
        }
        self.pending.insert(path, forced);
    }

    /// Put back on the device any subset of `pending`'s writes, each whole or torn, over the
    /// contents the last force left. Returns the writes it dropped and tore.
    fn lose(&mut self, files: &Files, path: &str, pending: Pending) -> Loss {
        let unit = self.faults.atomic_unit;
        let mut kept = Vec::new();
        let mut loss = Loss::default();
        for (offset, bytes) in pending.writes {
            let len = bytes.len() as u64;
            let before = kept.len();
            match decide_unforced(&mut self.rng, &self.faults, len, true) {
                Landing::Whole => kept.push((offset, bytes)),
                Landing::Lost => {}
                Landing::Torn(mask) => {
                    for (k, &lands) in (0u64..).zip(&mask) {
                        let at = k * unit;
                        if lands {
                            let end = (at + unit).min(len);
                            kept.push((offset + at, bytes[at as usize..end as usize].to_vec()));
                        }
                    }
                    if kept.len() > before && kept.len() - before < mask.len() {
                        loss.torn += 1;
                    }
                }
            }
            if kept.len() == before {
                loss.lost += 1;
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
        loss
    }

    /// Stop the device as a power loss would, losing what the faults allow.
    ///
    /// Returns the unforced writes it dropped and tore.
    pub(crate) fn crash(&mut self, files: &Files) -> Loss {
        self.crashed = true;
        let mut loss = Loss::default();
        for (path, pending) in std::mem::take(&mut self.pending) {
            let file = self.lose(files, &path, pending);
            loss.lost += file.lost;
            loss.torn += file.torn;
        }
        let keep = self.rng.below(self.entries.len() as u64 + 1) as usize;
        for entry in self.entries.drain(keep..).rev() {
            let r = match entry.undo {
                Undo::Create { path, dir: true } => fs::remove_dir_all(files.path(&path)),
                Undo::Create { path, dir: false } => fs::remove_file(files.path(&path)),
                Undo::Restore { path, bytes } => fs::write(files.path(&path), bytes),
                Undo::RemovedDir { path } => fs::create_dir(files.path(&path)),
                Undo::Renamed { from, to, over } => fs::rename(files.path(&to), files.path(&from))
                    .and_then(|()| match over {
                        Over::Nothing => Ok(()),
                        Over::EmptyDir => fs::create_dir(files.path(&to)),
                        Over::File(bytes) => fs::write(files.path(&to), bytes),
                    }),
            };
            match r {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    panic!("undo an unforced creation or removal: {e}")
                }
                _ => {}
            }
        }
        loss
    }
}
