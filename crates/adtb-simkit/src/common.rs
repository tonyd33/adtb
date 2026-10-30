use std::collections::{BTreeMap, BTreeSet};

use adtb::runtime::{AlignedBuf, Completion, Done, FileId, Io, IoError, Op, Runtime, Threads};
use adtb_sim::{Finding, FindingKind, Ledger, Recovered, SimIo, SimRuntime};

pub const SECTOR: usize = 512;

pub fn complete(rt: &SimRuntime, io: &mut SimIo, op: Op) -> Completion {
    let ticket = io.submit(op);
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            return c;
        }
        rt.park();
    }
}

pub fn open(
    rt: &SimRuntime,
    io: &mut SimIo,
    path: &str,
    create: bool,
) -> Result<(FileId, u64), IoError> {
    match complete(
        rt,
        io,
        Op::Open {
            path: path.into(),
            create,
        },
    )
    .result?
    {
        Done::Opened { file, len } => Ok((file, len)),
        other => panic!("open completed as {other:?}"),
    }
}

pub fn write(
    rt: &SimRuntime,
    io: &mut SimIo,
    file: FileId,
    offset: u64,
    data: &[u8],
) -> Result<(), IoError> {
    let c = complete(
        rt,
        io,
        Op::Write {
            file,
            offset,
            buf: AlignedBuf::from_slice(data),
        },
    );
    c.result.map(|d| assert_eq!(Done::Written, d))
}

pub fn read(
    rt: &SimRuntime,
    io: &mut SimIo,
    file: FileId,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, IoError> {
    let c = complete(
        rt,
        io,
        Op::Read {
            file,
            offset,
            buf: AlignedBuf::zeroed(len),
        },
    );
    match c.result? {
        Done::Read(n) => Ok(c.buf.unwrap()[..n].to_vec()),
        other => panic!("read completed as {other:?}"),
    }
}

pub fn sync(rt: &SimRuntime, io: &mut SimIo, file: FileId) -> Result<(), IoError> {
    complete(rt, io, Op::Sync { file })
        .result
        .map(|d| assert_eq!(Done::Synced, d))
}

pub fn sync_dir(rt: &SimRuntime, io: &mut SimIo, dir: &str) -> Result<(), IoError> {
    complete(rt, io, Op::SyncDir { dir: dir.into() })
        .result
        .map(|d| assert_eq!(Done::Synced, d))
}

pub fn allocate(rt: &SimRuntime, io: &mut SimIo, file: FileId, len: u64) -> Result<(), IoError> {
    complete(rt, io, Op::Allocate { file, len })
        .result
        .map(|d| assert_eq!(Done::Allocated, d))
}

pub fn sector(fill: u8) -> Vec<u8> {
    vec![fill; SECTOR]
}

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Returns the FNV digest of `bytes` continued from `digest`. Start from [`FNV_OFFSET`].
pub fn fnv(digest: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(digest, |d, &b| {
        (d ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

const MAGIC: &[u8; 8] = b"ADTBREC1";

/// Returns the one-sector record of commit `id`.
pub fn record(id: u64) -> Vec<u8> {
    let mut r = vec![0; SECTOR];
    r[..8].copy_from_slice(MAGIC);
    r[8..16].copy_from_slice(&id.to_le_bytes());
    let sum = checksum(&r);
    r[16..24].copy_from_slice(&sum.to_le_bytes());
    r
}

fn checksum(r: &[u8]) -> u64 {
    let mut copy = r.to_vec();
    copy[16..24].fill(0);
    fnv(FNV_OFFSET, &copy)
}

/// How a toy commit log acknowledges a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// Write the record, force it, then acknowledge.
    Forced,
    /// Write the record and acknowledge without forcing.
    Unforced,
    /// Write and force the record, then tell the client it aborted.
    AbortAfterWrite,
}

/// A toy commit log: commit `id` is one record in slot `id` of file `log`.
pub fn commit_all(rt: &SimRuntime, ledger: &Ledger, ids: &[u64], durability: Durability) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "log", true).unwrap();
    sync_dir(rt, &mut io, "").unwrap();
    for &id in ids {
        ledger.submit(id);
        write(rt, &mut io, file, id * SECTOR as u64, &record(id)).unwrap();
        match durability {
            Durability::Forced => {
                sync(rt, &mut io, file).unwrap();
                ledger.acknowledge(id);
            }
            Durability::Unforced => ledger.acknowledge(id),
            Durability::AbortAfterWrite => {
                sync(rt, &mut io, file).unwrap();
                ledger.abort(id);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Store {
    pub ids: BTreeSet<u64>,
    pub damaged: BTreeMap<u64, Vec<u8>>,
}

pub fn recover(rt: &SimRuntime) -> Store {
    let mut io = rt.io();
    let mut store = Store::default();
    let Ok((file, len)) = open(rt, &mut io, "log", false) else {
        return store;
    };
    if len == 0 {
        return store;
    }
    let bytes = read(
        rt,
        &mut io,
        file,
        0,
        len.next_multiple_of(SECTOR as u64) as usize,
    )
    .unwrap();
    for (slot, r) in bytes.chunks(SECTOR).enumerate() {
        if r.iter().all(|&b| b == 0) {
            continue;
        }
        let id = u64::from_le_bytes(r[8..16].try_into().unwrap());
        let sum = u64::from_le_bytes(r[16..24].try_into().unwrap());
        if &r[..8] == MAGIC && sum == checksum(r) && id == slot as u64 {
            store.ids.insert(id);
        } else {
            store.damaged.insert(slot as u64, r.to_vec());
        }
    }
    store
}

impl Recovered for Store {
    fn committed(&self) -> BTreeSet<u64> {
        self.ids.clone()
    }

    fn verify(&self) -> Vec<Finding> {
        self.damaged
            .keys()
            .map(|slot| Finding {
                kind: FindingKind::Structure,
                at: format!("log slot {slot}"),
            })
            .collect()
    }
}

pub fn sleep(rt: &SimRuntime, io: &mut SimIo, d: std::time::Duration) {
    use adtb::runtime::Clock;
    let at = rt.now() + d;
    assert_eq!(Ok(Done::Fired), complete(rt, io, Op::Timer { at }).result);
}

/// Run `body` and crash the process one second of simulated time after it starts, while it sleeps.
pub fn run_then_crash<F: FnOnce(&SimRuntime, &mut SimIo)>(sim: &mut adtb_sim::Sim, body: F) {
    sim.crash_at(sim.now() + std::time::Duration::from_secs(1));
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        body(&rt, &mut io);
        sleep(&rt, &mut io, std::time::Duration::from_secs(10));
        unreachable!("the crash comes first");
    });
    assert_eq!(adtb_sim::Outcome::Crashed, outcome);
}

pub fn run<T, F: FnOnce(&SimRuntime, &mut SimIo) -> T>(sim: &mut adtb_sim::Sim, body: F) -> T {
    let mut out = None;
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        out = Some(body(&rt, &mut io));
    });
    assert_eq!(adtb_sim::Outcome::Finished, outcome);
    out.unwrap()
}
