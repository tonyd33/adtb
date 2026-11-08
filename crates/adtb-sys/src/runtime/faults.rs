//! Decide the fate of each op under injected device faults.

use std::ops::Range;

/// The faults a backend injects beneath the engine's files, and the device geometry they act on.
///
/// Each `_p` is the chance of its fault per op that can suffer it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Faults {
    /// The seed of the real backend's generator. The simulator draws from its own seed.
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
    /// Chance that a write, an allocate or a force fails. A failed allocate changes nothing.
    pub fail_p: f64,
    /// Lose any subset of unforced writes, and any suffix of the unforced creations and removals,
    /// at a crash.
    pub lose_unsynced: bool,
    /// Model a device with no free space: a write past a file's end, or an allocate that grows a
    /// file, completes with `Full` and leaves the file as it was.
    pub no_space: bool,
    /// The bytes a latent error makes unreadable, a failed write lands in and a misdirected write
    /// is aligned to.
    pub sector: u64,
    /// The bytes the device writes atomically, a multiple of `sector`.
    pub atomic_unit: u64,
}

impl Faults {
    /// Returns a policy that injects no faults, over sectors of `sector` bytes each written
    /// atomically, losing unforced writes at a crash.
    pub const fn none(sector: u64) -> Faults {
        Faults {
            seed: 0,
            torn_p: 0.0,
            corrupt_p: 0.0,
            misdirect_p: 0.0,
            lost_write_p: 0.0,
            latent_p: 0.0,
            fail_p: 0.0,
            lose_unsynced: true,
            no_space: false,
            sector,
            atomic_unit: sector,
        }
    }
}

/// A source of the random draws a fault decision consumes.
pub trait Draw {
    /// Returns true with chance `p`, which is above zero.
    fn chance(&mut self, p: f64) -> bool;

    /// Returns a uniform draw from `0..n`, where `n` is above zero.
    fn below(&mut self, n: u64) -> u64;

    /// Returns true or false with even chance.
    fn coin(&mut self) -> bool;
}

/// Returns whether a fault of chance `p` strikes, drawing nothing when `p` is not above zero.
fn strikes(draw: &mut impl Draw, p: f64) -> bool {
    p > 0.0 && draw.chance(p)
}

/// What a backend does with a `Write`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteFate {
    /// Complete with `Full` and change nothing.
    Full,
    /// Land the buffer's bytes at these ranges of the file, then complete with `Failed`.
    Fail { landed: Vec<Range<u64>> },
    /// Complete as written and change nothing.
    Lost,
    /// Land the whole buffer at this offset and complete as written.
    At(u64),
}

/// Returns the fate of a write of `len` bytes at `offset` to a file of `file_len` bytes.
///
/// Postconditions:
///
/// - With `no_space`, a write past `file_len` is `Full` and draws nothing.
/// - Otherwise it draws fail, then one coin per sector of the write for whether it lands, then
///   lost, then misdirect, stopping at the first that strikes.
/// - A misdirected write lands at a sector-aligned offset other than its own, drawn uniformly
///   from those where it fits within the file; when there is none, it lands just past its end.
pub fn decide_write(
    draw: &mut impl Draw,
    faults: &Faults,
    offset: u64,
    len: u64,
    file_len: u64,
) -> WriteFate {
    if faults.no_space && offset + len > file_len {
        return WriteFate::Full;
    }
    if strikes(draw, faults.fail_p) {
        let mut landed = Vec::new();
        let mut at = offset;
        while at < offset + len {
            let end = (at + faults.sector).min(offset + len);
            if draw.coin() {
                landed.push(at..end);
            }
            at = end;
        }
        return WriteFate::Fail { landed };
    }
    if strikes(draw, faults.lost_write_p) {
        return WriteFate::Lost;
    }
    if !strikes(draw, faults.misdirect_p) {
        return WriteFate::At(offset);
    }
    let sector = faults.sector;
    let slots = (file_len.max(offset + len) - len) / sector + 1;
    if slots <= 1 {
        return WriteFate::At(offset + len);
    }
    let k = draw.below(slots - 1);
    WriteFate::At(if k >= offset / sector { k + 1 } else { k } * sector)
}

/// What a backend does with a `Read` that finds bytes in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFate {
    /// Make sector number `sector` unreadable until it is written again, and complete with
    /// `Failed`.
    Latent { sector: u64 },
    /// Complete with `Failed`: a sector of the range is unreadable already.
    Unreadable,
    /// Flip the bits `bit` of the stored byte at offset `byte`, then read.
    Corrupt { byte: u64, bit: u8 },
    /// Read.
    Clean,
}

/// Returns the fate of a read at `offset` that finds `n` bytes in the file, `unreadable` when a
/// sector of those bytes is unreadable already.
///
/// Postconditions:
///
/// - A read that finds no bytes is `Clean` and draws nothing.
/// - Otherwise it draws latent, and its sector; then, unless a sector is unreadable, corrupt, its
///   byte and its bit.
pub fn decide_read(
    draw: &mut impl Draw,
    faults: &Faults,
    offset: u64,
    n: u64,
    unreadable: bool,
) -> ReadFate {
    if n == 0 {
        return ReadFate::Clean;
    }
    let sector = faults.sector;
    if strikes(draw, faults.latent_p) {
        let first = offset / sector;
        let last = (offset + n - 1) / sector;
        return ReadFate::Latent {
            sector: first + draw.below(last - first + 1),
        };
    }
    if unreadable {
        return ReadFate::Unreadable;
    }
    if !strikes(draw, faults.corrupt_p) {
        return ReadFate::Clean;
    }
    let byte = offset + draw.below(n);
    let bit = 1u8 << draw.below(8);
    ReadFate::Corrupt { byte, bit }
}

/// What a backend does with an `Allocate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocateFate {
    /// Complete with `Failed` and change nothing.
    Fail,
    /// Complete with `Full` and change nothing.
    Full,
    /// Allocate.
    Run,
}

/// Returns the fate of an allocate to `len` bytes of a file of `file_len` bytes.
///
/// Postconditions:
///
/// - It draws fail; when fail does not strike, an allocate that grows the file under `no_space`
///   is `Full`.
pub fn decide_allocate(
    draw: &mut impl Draw,
    faults: &Faults,
    len: u64,
    file_len: u64,
) -> AllocateFate {
    if strikes(draw, faults.fail_p) {
        AllocateFate::Fail
    } else if faults.no_space && len > file_len {
        AllocateFate::Full
    } else {
        AllocateFate::Run
    }
}

/// What a backend does with a `Sync` or a `SyncDir`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceFate {
    /// Complete with `Failed`.
    Fail,
    /// Force.
    Run,
}

/// Returns the fate of a `Sync` or a `SyncDir`, drawing fail.
pub fn decide_force(draw: &mut impl Draw, faults: &Faults) -> ForceFate {
    if strikes(draw, faults.fail_p) {
        ForceFate::Fail
    } else {
        ForceFate::Run
    }
}

/// What a crash leaves of one write no force covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The whole write is on the device.
    Whole,
    /// None of the write is on the device.
    Lost,
    /// Each atomic unit of the write, in order, is on the device where its flag is set.
    Torn(Vec<bool>),
}

/// Returns what a crash leaves of an unforced write of `len` bytes, `completed` when the write
/// completed before the crash and was not still in flight.
///
/// Postconditions:
///
/// - A completed write without `lose_unsynced` is `Whole` and draws nothing.
/// - Otherwise a write larger than the atomic unit draws torn, and when it strikes, one coin per
///   atomic unit for whether it lands; a write that is not torn draws one coin for whether it
///   lands whole.
pub fn decide_unforced(
    draw: &mut impl Draw,
    faults: &Faults,
    len: u64,
    completed: bool,
) -> Landing {
    if completed && !faults.lose_unsynced {
        return Landing::Whole;
    }
    if len > faults.atomic_unit && strikes(draw, faults.torn_p) {
        let units = len.div_ceil(faults.atomic_unit);
        return Landing::Torn((0..units).map(|_| draw.coin()).collect());
    }
    if draw.coin() {
        Landing::Whole
    } else {
        Landing::Lost
    }
}
