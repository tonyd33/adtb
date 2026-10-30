use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::ops::Range;

use adtb::runtime::IoError;

use crate::rng::Rng;

/// The probabilities and switches of the faults `SimStorage` injects.
#[derive(Clone, Debug)]
pub struct Faults {
    /// Chance that a write left unsynced at a crash, and larger than the atomic unit, lands torn.
    pub torn_p: f64,
    /// Chance that a read flips one stored bit in its range first.
    pub corrupt_p: f64,
    /// Chance that a write lands at another offset of the same file.
    pub misdirect_p: f64,
    /// Chance that a write reports success and never reaches the device.
    pub lost_write_p: f64,
    /// Chance that a read makes one sector in its range unreadable until it is rewritten.
    pub latent_p: f64,
    /// Chance that a write or force fails.
    pub fail_p: f64,
    /// Lose any subset of unforced writes, creations and removals at a crash.
    pub lose_unsynced: bool,
    /// Complete ops after random delays, and so out of submission order.
    pub delay_completions: bool,
    /// The device has no free space: a write past a file's end, or an allocate that grows a
    /// file, fails with `Full` and changes nothing.
    pub no_space: bool,
}

impl Default for Faults {
    fn default() -> Self {
        Faults {
            torn_p: 0.0,
            corrupt_p: 0.0,
            misdirect_p: 0.0,
            lost_write_p: 0.0,
            latent_p: 0.0,
            fail_p: 0.0,
            lose_unsynced: true,
            delay_completions: true,
            no_space: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub(crate) sector: u64,
    pub(crate) atomic_unit: u64,
}

pub(crate) type Ino = usize;

/// The bytes of one write. A run of zeros is held by its length.
#[derive(Clone, Debug)]
enum Payload {
    Bytes(Vec<u8>),
    Zeros(usize),
}

impl Payload {
    fn len(&self) -> usize {
        match self {
            Payload::Bytes(data) => data.len(),
            Payload::Zeros(len) => *len,
        }
    }

    /// Store the whole payload at `offset`.
    fn land(&self, bytes: &mut Vec<u8>, offset: u64) {
        self.land_part(bytes, offset, 0..self.len());
    }

    /// Store `part` of the payload where it falls when the whole payload starts at `offset`.
    fn land_part(&self, bytes: &mut Vec<u8>, offset: u64, part: Range<usize>) {
        let at = offset + part.start as u64;
        match self {
            Payload::Bytes(data) => put(bytes, at, &data[part]),
            Payload::Zeros(_) => put_zeros(bytes, at, part.len()),
        }
    }
}

#[derive(Debug)]
struct Unsynced {
    seq: u64,
    offset: u64,
    data: Payload,
}

#[derive(Debug, Default)]
struct Inode {
    visible: Vec<u8>,
    durable: Vec<u8>,
    unsynced: Vec<Unsynced>,
    bad: BTreeSet<u64>,
}

#[derive(Debug)]
enum NameOp {
    Create(String, Ino),
    Remove(String),
    MakeDir(String),
}

impl NameOp {
    fn path(&self) -> &str {
        match self {
            NameOp::Create(p, _) | NameOp::Remove(p) | NameOp::MakeDir(p) => p,
        }
    }
}

/// A write the crash finds still in flight.
pub(crate) struct InFlight {
    pub(crate) ino: Ino,
    pub(crate) offset: u64,
    pub(crate) data: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct Disk {
    inodes: Vec<Inode>,
    names: BTreeMap<String, Ino>,
    durable_names: BTreeMap<String, Ino>,
    dirs: BTreeSet<String>,
    durable_dirs: BTreeSet<String>,
    pending_names: Vec<(u64, NameOp)>,
    seq: u64,
    recording: Option<Recording>,
}

#[derive(Clone, Debug)]
enum Event {
    Write {
        ino: Ino,
        offset: u64,
        data: Payload,
        seq: u64,
    },
    Create {
        path: String,
        ino: Ino,
        seq: u64,
    },
    Remove {
        path: String,
        seq: u64,
    },
    MakeDir {
        path: String,
        seq: u64,
    },
    Sync {
        ino: Ino,
        upto: u64,
    },
    SyncDir {
        dir: String,
        upto: u64,
    },
}

/// The stream of writes, name changes and forces that reached the device, from a durable base.
#[derive(Clone, Debug, Default)]
pub struct Recording {
    base_inodes: Vec<Vec<u8>>,
    base_names: BTreeMap<String, Ino>,
    base_dirs: BTreeSet<String>,
    events: Vec<Event>,
}

pub(crate) struct Ctx<'a> {
    pub(crate) rng: &'a mut Rng,
    pub(crate) faults: &'a Faults,
    pub(crate) geometry: Geometry,
    pub(crate) trace: &'a mut String,
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn dir_exists(dirs: &BTreeSet<String>, dir: &str) -> bool {
    dir.is_empty() || dirs.contains(dir)
}

/// Keep only the directories and files whose every ancestor directory is kept.
fn prune(dirs: &mut BTreeSet<String>, names: &mut BTreeMap<String, Ino>) {
    let mut kept = BTreeSet::new();
    for d in std::mem::take(dirs) {
        if dir_exists(&kept, parent(&d)) {
            kept.insert(d);
        }
    }
    names.retain(|p, _| dir_exists(&kept, parent(p)));
    *dirs = kept;
}

fn put(bytes: &mut Vec<u8>, offset: u64, data: &[u8]) {
    let start = offset as usize;
    let end = start + data.len();
    if bytes.len() < end {
        bytes.resize(end, 0);
    }
    bytes[start..end].copy_from_slice(data);
}

fn put_zeros(bytes: &mut Vec<u8>, offset: u64, len: usize) {
    let start = offset as usize;
    let end = start + len;
    assert!(
        bytes.len() <= start,
        "zeros extend a file: they land at or past its end, not at {start} of {}",
        bytes.len()
    );
    if bytes.is_empty() {
        // `vec!` takes untouched zero pages from the allocator; `resize` would touch every one.
        *bytes = vec![0; end];
    } else {
        bytes.resize(end, 0);
    }
}

impl Disk {
    pub(crate) fn from_files(files: BTreeMap<String, Vec<u8>>) -> Disk {
        let mut disk = Disk::default();
        for (path, bytes) in files {
            disk.names.insert(path, disk.inodes.len());
            disk.inodes.push(Inode {
                visible: bytes.clone(),
                durable: bytes,
                ..Inode::default()
            });
        }
        for path in disk.names.keys() {
            let mut dir = parent(path);
            while !dir.is_empty() {
                disk.dirs.insert(dir.to_owned());
                dir = parent(dir);
            }
        }
        disk.durable_names = disk.names.clone();
        disk.durable_dirs = disk.dirs.clone();
        disk
    }

    /// Start recording, taking every write and name change so far as forced.
    pub(crate) fn record(&mut self) {
        self.recording = Some(Recording {
            base_inodes: self.inodes.iter().map(|i| i.visible.clone()).collect(),
            base_names: self.names.clone(),
            base_dirs: self.dirs.clone(),
            events: Vec::new(),
        });
    }

    pub(crate) fn recording(&self) -> Option<&Recording> {
        self.recording.as_ref()
    }

    fn note(&mut self, event: Event) {
        if let Some(r) = &mut self.recording {
            r.events.push(event);
        }
    }

    fn path_of(&self, ino: Ino) -> String {
        self.names
            .iter()
            .chain(&self.durable_names)
            .find(|(_, i)| **i == ino)
            .map_or_else(String::new, |(p, _)| p.clone())
    }

    /// Returns the sequence number the next completed write or name change gets.
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    pub(crate) fn files(&self) -> BTreeMap<String, Vec<u8>> {
        self.names
            .iter()
            .map(|(path, &ino)| (path.clone(), self.inodes[ino].visible.clone()))
            .collect()
    }

    pub(crate) fn open(&mut self, path: &str, create: bool) -> Result<(Ino, u64), IoError> {
        if let Some(&ino) = self.names.get(path) {
            return Ok((ino, self.inodes[ino].visible.len() as u64));
        }
        if !create || !dir_exists(&self.dirs, parent(path)) {
            return Err(IoError::NotFound);
        }
        let ino = self.inodes.len();
        self.inodes.push(Inode::default());
        self.names.insert(path.to_owned(), ino);
        self.pending_names
            .push((self.seq, NameOp::Create(path.to_owned(), ino)));
        self.note(Event::Create {
            path: path.to_owned(),
            ino,
            seq: self.seq,
        });
        self.seq += 1;
        Ok((ino, 0))
    }

    pub(crate) fn remove(&mut self, path: &str) -> Result<(), IoError> {
        if self.names.remove(path).is_none() {
            return Err(IoError::NotFound);
        }
        self.pending_names
            .push((self.seq, NameOp::Remove(path.to_owned())));
        self.note(Event::Remove {
            path: path.to_owned(),
            seq: self.seq,
        });
        self.seq += 1;
        Ok(())
    }

    pub(crate) fn create_dir(&mut self, dir: &str) -> Result<(), IoError> {
        if dir_exists(&self.dirs, dir) {
            return Ok(());
        }
        if !dir_exists(&self.dirs, parent(dir)) {
            return Err(IoError::NotFound);
        }
        self.dirs.insert(dir.to_owned());
        self.pending_names
            .push((self.seq, NameOp::MakeDir(dir.to_owned())));
        self.note(Event::MakeDir {
            path: dir.to_owned(),
            seq: self.seq,
        });
        self.seq += 1;
        Ok(())
    }

    pub(crate) fn size(&self, ino: Ino) -> u64 {
        self.inodes[ino].visible.len() as u64
    }

    pub(crate) fn list(&self, dir: &str, rng: &mut Rng) -> Result<Vec<String>, IoError> {
        if !dir_exists(&self.dirs, dir) {
            return Err(IoError::NotFound);
        }
        let mut out: Vec<String> = self
            .names
            .keys()
            .filter(|p| parent(p) == dir)
            .cloned()
            .collect();
        for i in (1..out.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            out.swap(i, j);
        }
        Ok(out)
    }

    pub(crate) fn read(
        &mut self,
        ino: Ino,
        offset: u64,
        buf: &mut [u8],
        cx: Ctx<'_>,
    ) -> Result<usize, IoError> {
        let sector = cx.geometry.sector;
        assert!(
            offset.is_multiple_of(sector) && (buf.len() as u64).is_multiple_of(sector),
            "read at {offset} of {} bytes is not sector aligned",
            buf.len()
        );
        let path = self.path_of(ino);
        let inode = &mut self.inodes[ino];
        let len = inode.visible.len() as u64;
        let end = (offset + buf.len() as u64).min(len);
        if offset >= end {
            return Ok(0);
        }
        if cx.rng.chance(cx.faults.latent_p) {
            let first = offset / sector;
            let last = (end - 1) / sector;
            let s = cx.rng.between(first, last);
            inode.bad.insert(s);
            trace!(cx.trace, "fault latent ino {ino} {path:?} sector {s}");
        }
        if inode
            .bad
            .range(offset / sector..=(end - 1) / sector)
            .next()
            .is_some()
        {
            trace!(cx.trace, "read ino {ino} @{offset} hits a bad sector");
            return Err(IoError::Failed);
        }
        if cx.rng.chance(cx.faults.corrupt_p) {
            let at = cx.rng.between(offset, end - 1) as usize;
            let bit = 1u8 << cx.rng.below(8);
            inode.visible[at] ^= bit;
            if let Some(b) = inode.durable.get_mut(at) {
                *b ^= bit;
            }
            trace!(
                cx.trace,
                "fault corrupt ino {ino} {path:?} byte {at} bit {bit:#04x}"
            );
        }
        let n = (end - offset) as usize;
        buf[..n].copy_from_slice(&inode.visible[offset as usize..end as usize]);
        Ok(n)
    }

    pub(crate) fn write(
        &mut self,
        ino: Ino,
        offset: u64,
        data: &[u8],
        cx: Ctx<'_>,
    ) -> Result<(), IoError> {
        let sector = cx.geometry.sector;
        assert!(
            offset.is_multiple_of(sector) && (data.len() as u64).is_multiple_of(sector),
            "write at {offset} of {} bytes is not sector aligned",
            data.len()
        );
        let path = self.path_of(ino);
        if cx.faults.no_space && offset + data.len() as u64 > self.inodes[ino].visible.len() as u64
        {
            trace!(cx.trace, "no space for write ino {ino} {path:?} @{offset}");
            return Err(IoError::Full);
        }
        if cx.rng.chance(cx.faults.fail_p) {
            let mut landed = Vec::new();
            for (i, chunk) in data.chunks(sector as usize).enumerate() {
                if cx.rng.coin() {
                    let at = offset + i as u64 * sector;
                    self.apply(ino, at, Payload::Bytes(chunk.to_vec()), sector);
                    landed.push(i);
                }
            }
            trace!(
                cx.trace,
                "fault fail write ino {ino} {path:?} @{offset} landed sectors {landed:?}"
            );
            return Err(IoError::Failed);
        }
        if cx.rng.chance(cx.faults.lost_write_p) {
            trace!(cx.trace, "fault lost write ino {ino} {path:?} @{offset}");
            return Ok(());
        }
        let mut target = offset;
        if cx.rng.chance(cx.faults.misdirect_p) {
            target = self.misdirect(ino, offset, data.len() as u64, sector, cx.rng);
            trace!(
                cx.trace,
                "fault misdirect ino {ino} {path:?} @{offset} to @{target}"
            );
        }
        self.apply(ino, target, Payload::Bytes(data.to_vec()), sector);
        Ok(())
    }

    fn misdirect(&self, ino: Ino, offset: u64, len: u64, sector: u64, rng: &mut Rng) -> u64 {
        let file_len = (self.inodes[ino].visible.len() as u64).max(offset + len);
        let slots = (file_len - len) / sector + 1;
        if slots <= 1 {
            return offset + len;
        }
        let original = offset / sector;
        let k = rng.below(slots - 1);
        let k = if k >= original { k + 1 } else { k };
        k * sector
    }

    fn apply(&mut self, ino: Ino, offset: u64, data: Payload, sector: u64) {
        let inode = &mut self.inodes[ino];
        data.land(&mut inode.visible, offset);
        let first = offset / sector;
        let last = (offset + data.len() as u64).div_ceil(sector);
        let healed: Vec<u64> = inode.bad.range(first..last).copied().collect();
        for s in healed {
            inode.bad.remove(&s);
        }
        if let Some(recording) = &mut self.recording {
            recording.events.push(Event::Write {
                ino,
                offset,
                data: data.clone(),
                seq: self.seq,
            });
        }
        inode.unsynced.push(Unsynced {
            seq: self.seq,
            offset,
            data,
        });
        self.seq += 1;
    }

    pub(crate) fn allocate(&mut self, ino: Ino, len: u64, cx: Ctx<'_>) -> Result<(), IoError> {
        let have = self.inodes[ino].visible.len() as u64;
        if len > have {
            if cx.faults.no_space {
                trace!(cx.trace, "no space to allocate ino {ino} to {len}");
                return Err(IoError::Full);
            }
            let zeros = Payload::Zeros((len - have) as usize);
            self.apply(ino, have, zeros, cx.geometry.sector);
        }
        Ok(())
    }

    pub(crate) fn sync(&mut self, ino: Ino, upto: u64, cx: Ctx<'_>) -> Result<(), IoError> {
        if cx.rng.chance(cx.faults.fail_p) {
            let path = self.path_of(ino);
            trace!(cx.trace, "fault fail sync ino {ino} {path:?}");
            return Err(IoError::Failed);
        }
        let inode = &mut self.inodes[ino];
        let (forced, kept): (Vec<_>, Vec<_>) = inode.unsynced.drain(..).partition(|w| w.seq < upto);
        inode.unsynced = kept;
        for w in forced {
            w.data.land(&mut inode.durable, w.offset);
        }
        self.note(Event::Sync { ino, upto });
        Ok(())
    }

    pub(crate) fn sync_dir(&mut self, dir: &str, upto: u64, cx: Ctx<'_>) -> Result<(), IoError> {
        if cx.rng.chance(cx.faults.fail_p) {
            trace!(cx.trace, "fault fail sync dir {dir:?}");
            return Err(IoError::Failed);
        }
        let (forced, kept): (Vec<_>, Vec<_>) = self
            .pending_names
            .drain(..)
            .partition(|(seq, op)| *seq < upto && parent(op.path()) == dir);
        self.pending_names = kept;
        for (_, op) in forced {
            apply_name(&mut self.durable_names, &mut self.durable_dirs, op);
        }
        self.note(Event::SyncDir {
            dir: dir.to_owned(),
            upto,
        });
        Ok(())
    }

    /// Reduce the disk to what a crash at this instant leaves on the device.
    pub(crate) fn crash(&mut self, in_flight: Vec<InFlight>, cx: Ctx<'_>) {
        let Ctx {
            rng,
            faults,
            geometry,
            trace,
        } = cx;
        let mut landings: Vec<(Ino, u64, Payload, bool)> = Vec::new();
        for (ino, inode) in self.inodes.iter_mut().enumerate() {
            for w in inode.unsynced.drain(..) {
                landings.push((ino, w.offset, w.data, true));
            }
        }
        landings.extend(
            in_flight
                .into_iter()
                .map(|w| (w.ino, w.offset, Payload::Bytes(w.data), false)),
        );
        for (ino, offset, data, acked) in landings {
            let durable = &mut self.inodes[ino].durable;
            if acked && !faults.lose_unsynced {
                data.land(durable, offset);
                continue;
            }
            let unit = geometry.atomic_unit as usize;
            if data.len() > unit && rng.chance(faults.torn_p) {
                let mut mask = Vec::new();
                for start in (0..data.len()).step_by(unit) {
                    let lands = rng.coin();
                    if lands {
                        let end = (start + unit).min(data.len());
                        data.land_part(durable, offset, start..end);
                    }
                    mask.push(lands);
                }
                trace!(trace, "crash torn ino {ino} @{offset} units {mask:?}");
            } else if rng.coin() {
                data.land(durable, offset);
                trace!(trace, "crash landed ino {ino} @{offset}");
            } else {
                trace!(trace, "crash lost ino {ino} @{offset}");
            }
        }
        for inode in &mut self.inodes {
            inode.visible = inode.durable.clone();
        }
        let pending = std::mem::take(&mut self.pending_names);
        let keep = if faults.lose_unsynced {
            rng.below(pending.len() as u64 + 1) as usize
        } else {
            pending.len()
        };
        trace!(
            trace,
            "crash keeps {keep} of {} name changes",
            pending.len()
        );
        for (_, op) in pending.into_iter().take(keep) {
            apply_name(&mut self.durable_names, &mut self.durable_dirs, op);
        }
        prune(&mut self.durable_dirs, &mut self.durable_names);
        self.names = self.durable_names.clone();
        self.dirs = self.durable_dirs.clone();
    }
}

fn apply_name(names: &mut BTreeMap<String, Ino>, dirs: &mut BTreeSet<String>, op: NameOp) {
    match op {
        NameOp::Create(path, ino) => {
            names.insert(path, ino);
        }
        NameOp::Remove(path) => {
            names.remove(&path);
        }
        NameOp::MakeDir(path) => {
            dirs.insert(path);
        }
    }
}

impl Recording {
    /// Returns the number of events recorded.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns each prefix length of the recording that ends with a completed force.
    pub fn flushes(&self) -> Vec<usize> {
        self.events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Event::Sync { .. } | Event::SyncDir { .. }))
            .map(|(i, _)| i + 1)
            .collect()
    }

    /// Returns disk images a crash after the first `at` events may leave: the forced writes and
    /// name changes, plus each subset of the rest, or `max` random subsets when there are more.
    pub fn images(&self, at: usize, max: usize, seed: u64) -> Vec<BTreeMap<String, Vec<u8>>> {
        let events = &self.events[..at];
        let forced = |i: usize| -> bool {
            let later = &events[i..];
            match &events[i] {
                Event::Write { ino, seq, .. } => later.iter().any(|e| {
                    matches!(e, Event::Sync { ino: j, upto } if j == ino && upto > seq)
                }),
                Event::Create { path, seq, .. }
                | Event::Remove { path, seq }
                | Event::MakeDir { path, seq } => {
                    later.iter().any(|e| {
                        matches!(e, Event::SyncDir { dir, upto } if dir == parent(path) && upto > seq)
                    })
                }
                Event::Sync { .. } | Event::SyncDir { .. } => false,
            }
        };
        let open: Vec<usize> = (0..at)
            .filter(|&i| !matches!(events[i], Event::Sync { .. } | Event::SyncDir { .. }))
            .filter(|&i| !forced(i))
            .collect();
        let mut subsets: Vec<Vec<bool>> = Vec::new();
        if open.len() < 63 && (1u64 << open.len()) <= max as u64 {
            for mask in 0..(1u64 << open.len()) {
                subsets.push((0..open.len()).map(|b| mask >> b & 1 == 1).collect());
            }
        } else {
            let mut rng = Rng::new(seed);
            subsets.push(vec![false; open.len()]);
            subsets.push(vec![true; open.len()]);
            while subsets.len() < max {
                subsets.push((0..open.len()).map(|_| rng.coin()).collect());
            }
        }
        subsets
            .into_iter()
            .map(|chosen| {
                let mut keep = vec![true; at];
                for (k, &i) in open.iter().enumerate() {
                    keep[i] = chosen[k];
                }
                self.image(events, &keep)
            })
            .collect()
    }

    fn image(&self, events: &[Event], keep: &[bool]) -> BTreeMap<String, Vec<u8>> {
        let mut inodes = self.base_inodes.clone();
        let mut names = self.base_names.clone();
        let mut dirs = self.base_dirs.clone();
        for (e, &k) in events.iter().zip(keep) {
            if !k {
                continue;
            }
            match e {
                Event::Write {
                    ino, offset, data, ..
                } => {
                    if inodes.len() <= *ino {
                        inodes.resize(*ino + 1, Vec::new());
                    }
                    data.land(&mut inodes[*ino], *offset);
                }
                Event::Create { path, ino, .. } => {
                    names.insert(path.clone(), *ino);
                }
                Event::Remove { path, .. } => {
                    names.remove(path);
                }
                Event::MakeDir { path, .. } => {
                    dirs.insert(path.clone());
                }
                Event::Sync { .. } | Event::SyncDir { .. } => {}
            }
        }
        prune(&mut dirs, &mut names);
        names
            .into_iter()
            .map(|(path, ino)| (path, inodes.get(ino).cloned().unwrap_or_default()))
            .collect()
    }
}
