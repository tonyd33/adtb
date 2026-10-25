use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

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
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub(crate) sector: u64,
    pub(crate) atomic_unit: u64,
}

pub(crate) type Ino = usize;

#[derive(Debug)]
struct Unsynced {
    seq: u64,
    offset: u64,
    data: Vec<u8>,
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
}

impl NameOp {
    fn path(&self) -> &str {
        match self {
            NameOp::Create(p, _) | NameOp::Remove(p) => p,
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
    pending_names: Vec<(u64, NameOp)>,
    seq: u64,
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

fn put(bytes: &mut Vec<u8>, offset: u64, data: &[u8]) {
    let start = offset as usize;
    let end = start + data.len();
    if bytes.len() < end {
        bytes.resize(end, 0);
    }
    bytes[start..end].copy_from_slice(data);
}

impl Disk {
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
        if !create {
            return Err(IoError::NotFound);
        }
        let ino = self.inodes.len();
        self.inodes.push(Inode::default());
        self.names.insert(path.to_owned(), ino);
        self.pending_names
            .push((self.seq, NameOp::Create(path.to_owned(), ino)));
        self.seq += 1;
        Ok((ino, 0))
    }

    pub(crate) fn remove(&mut self, path: &str) -> Result<(), IoError> {
        if self.names.remove(path).is_none() {
            return Err(IoError::NotFound);
        }
        self.pending_names
            .push((self.seq, NameOp::Remove(path.to_owned())));
        self.seq += 1;
        Ok(())
    }

    pub(crate) fn list(&self, dir: &str, rng: &mut Rng) -> Vec<String> {
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
        out
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
            let _ = writeln!(cx.trace, "fault latent ino {ino} sector {s}");
        }
        if inode
            .bad
            .range(offset / sector..=(end - 1) / sector)
            .next()
            .is_some()
        {
            let _ = writeln!(cx.trace, "read ino {ino} @{offset} hits a bad sector");
            return Err(IoError::Failed);
        }
        if cx.rng.chance(cx.faults.corrupt_p) {
            let at = cx.rng.between(offset, end - 1) as usize;
            let bit = 1u8 << cx.rng.below(8);
            inode.visible[at] ^= bit;
            if let Some(b) = inode.durable.get_mut(at) {
                *b ^= bit;
            }
            let _ = writeln!(cx.trace, "fault corrupt ino {ino} byte {at} bit {bit:#04x}");
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
        if cx.rng.chance(cx.faults.fail_p) {
            let mut landed = Vec::new();
            for (i, chunk) in data.chunks(sector as usize).enumerate() {
                if cx.rng.coin() {
                    let at = offset + i as u64 * sector;
                    self.apply(ino, at, chunk, sector);
                    landed.push(i);
                }
            }
            let _ = writeln!(
                cx.trace,
                "fault fail write ino {ino} @{offset} landed sectors {landed:?}"
            );
            return Err(IoError::Failed);
        }
        if cx.rng.chance(cx.faults.lost_write_p) {
            let _ = writeln!(cx.trace, "fault lost write ino {ino} @{offset}");
            return Ok(());
        }
        let mut target = offset;
        if cx.rng.chance(cx.faults.misdirect_p) {
            target = self.misdirect(ino, offset, data.len() as u64, sector, cx.rng);
            let _ = writeln!(cx.trace, "fault misdirect ino {ino} @{offset} to @{target}");
        }
        self.apply(ino, target, data, sector);
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

    fn apply(&mut self, ino: Ino, offset: u64, data: &[u8], sector: u64) {
        let inode = &mut self.inodes[ino];
        put(&mut inode.visible, offset, data);
        let first = offset / sector;
        let last = (offset + data.len() as u64).div_ceil(sector);
        let healed: Vec<u64> = inode.bad.range(first..last).copied().collect();
        for s in healed {
            inode.bad.remove(&s);
        }
        inode.unsynced.push(Unsynced {
            seq: self.seq,
            offset,
            data: data.to_vec(),
        });
        self.seq += 1;
    }

    pub(crate) fn allocate(&mut self, ino: Ino, len: u64, sector: u64) {
        let have = self.inodes[ino].visible.len() as u64;
        if len > have {
            let zeros = vec![0; (len - have) as usize];
            self.apply(ino, have, &zeros, sector);
        }
    }

    pub(crate) fn sync(&mut self, ino: Ino, upto: u64, cx: Ctx<'_>) -> Result<(), IoError> {
        if cx.rng.chance(cx.faults.fail_p) {
            let _ = writeln!(cx.trace, "fault fail sync ino {ino}");
            return Err(IoError::Failed);
        }
        let inode = &mut self.inodes[ino];
        let (forced, kept): (Vec<_>, Vec<_>) = inode.unsynced.drain(..).partition(|w| w.seq < upto);
        inode.unsynced = kept;
        for w in forced {
            put(&mut inode.durable, w.offset, &w.data);
        }
        Ok(())
    }

    pub(crate) fn sync_dir(&mut self, dir: &str, upto: u64, cx: Ctx<'_>) -> Result<(), IoError> {
        if cx.rng.chance(cx.faults.fail_p) {
            let _ = writeln!(cx.trace, "fault fail sync dir {dir:?}");
            return Err(IoError::Failed);
        }
        let (forced, kept): (Vec<_>, Vec<_>) = self
            .pending_names
            .drain(..)
            .partition(|(seq, op)| *seq < upto && parent(op.path()) == dir);
        self.pending_names = kept;
        for (_, op) in forced {
            apply_name(&mut self.durable_names, op);
        }
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
        let mut landings: Vec<(Ino, u64, Vec<u8>, bool)> = Vec::new();
        for (ino, inode) in self.inodes.iter_mut().enumerate() {
            for w in inode.unsynced.drain(..) {
                landings.push((ino, w.offset, w.data, true));
            }
        }
        landings.extend(
            in_flight
                .into_iter()
                .map(|w| (w.ino, w.offset, w.data, false)),
        );
        for (ino, offset, data, acked) in landings {
            let durable = &mut self.inodes[ino].durable;
            if acked && !faults.lose_unsynced {
                put(durable, offset, &data);
                continue;
            }
            let unit = geometry.atomic_unit as usize;
            if data.len() > unit && rng.chance(faults.torn_p) {
                let mut mask = Vec::new();
                for (i, chunk) in data.chunks(unit).enumerate() {
                    let lands = rng.coin();
                    if lands {
                        put(durable, offset + (i * unit) as u64, chunk);
                    }
                    mask.push(lands);
                }
                let _ = writeln!(trace, "crash torn ino {ino} @{offset} units {mask:?}");
            } else if rng.coin() {
                put(durable, offset, &data);
                let _ = writeln!(trace, "crash landed ino {ino} @{offset}");
            } else {
                let _ = writeln!(trace, "crash lost ino {ino} @{offset}");
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
        let _ = writeln!(
            trace,
            "crash keeps {keep} of {} name changes",
            pending.len()
        );
        for (_, op) in pending.into_iter().take(keep) {
            apply_name(&mut self.durable_names, op);
        }
        self.names = self.durable_names.clone();
    }
}

fn apply_name(names: &mut BTreeMap<String, Ino>, op: NameOp) {
    match op {
        NameOp::Create(path, ino) => {
            names.insert(path, ino);
        }
        NameOp::Remove(path) => {
            names.remove(&path);
        }
    }
}
