//! The runtime contract every backend meets, run against the simulator and the host.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adtb::runtime::{AlignedBuf, Completion, Done, FileId, Io, IoError, Op, Runtime, Ticket};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_sys::os::{Backend, Faults, OsRuntime};

#[path = "io_contract/faults.rs"]
mod faults;

const BLOCK: usize = 4096;

fn complete<R: Runtime>(rt: &R, io: &mut R::Io, op: Op) -> Completion {
    let ticket = io.submit(op);
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            return c;
        }
        rt.park();
    }
}

fn done<R: Runtime>(rt: &R, io: &mut R::Io, op: Op) -> Result<Done, IoError> {
    complete(rt, io, op).result
}

fn open<R: Runtime>(
    rt: &R,
    io: &mut R::Io,
    path: &str,
    create: bool,
) -> Result<(FileId, u64), IoError> {
    let path = path.into();
    match done(rt, io, Op::Open { path, create })? {
        Done::Opened { file, len } => Ok((file, len)),
        other => panic!("open completed as {other:?}"),
    }
}

fn write<R: Runtime>(rt: &R, io: &mut R::Io, file: FileId, offset: u64, data: &[u8]) {
    let buf = AlignedBuf::from_slice(data);
    assert_eq!(
        Ok(Done::Written),
        done(rt, io, Op::Write { file, offset, buf })
    );
}

fn read<R: Runtime>(rt: &R, io: &mut R::Io, file: FileId, offset: u64, len: usize) -> Vec<u8> {
    let buf = AlignedBuf::zeroed(len);
    let c = complete(rt, io, Op::Read { file, offset, buf });
    match c.result {
        Ok(Done::Read(n)) => c.buf.expect("a read returns its buffer")[..n].to_vec(),
        other => panic!("read completed as {other:?}"),
    }
}

fn size<R: Runtime>(rt: &R, io: &mut R::Io, file: FileId) -> u64 {
    match done(rt, io, Op::Size { file }) {
        Ok(Done::Size(n)) => n,
        other => panic!("size completed as {other:?}"),
    }
}

fn list<R: Runtime>(rt: &R, io: &mut R::Io, dir: &str) -> Result<BTreeSet<String>, IoError> {
    match done(rt, io, Op::List { dir: dir.into() })? {
        Done::Listed(paths) => Ok(paths.into_iter().collect()),
        other => panic!("list completed as {other:?}"),
    }
}

fn create_dir<R: Runtime>(rt: &R, io: &mut R::Io, dir: &str) -> Result<Done, IoError> {
    done(rt, io, Op::CreateDir { dir: dir.into() })
}

fn block(fill: u8) -> Vec<u8> {
    vec![fill; BLOCK]
}

fn write_then_read<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, len) = open(rt, &mut io, "f", true).unwrap();
    assert_eq!(0, len);
    write(rt, &mut io, file, 0, &block(1));
    write(rt, &mut io, file, 2 * BLOCK as u64, &block(3));
    let bytes = read(rt, &mut io, file, 0, 3 * BLOCK);
    assert_eq!(block(1), bytes[..BLOCK]);
    assert_eq!(vec![0; BLOCK], bytes[BLOCK..2 * BLOCK]);
    assert_eq!(block(3), bytes[2 * BLOCK..]);
    assert_eq!(Ok(Done::Synced), done(rt, &mut io, Op::Sync { file }));
}

fn open_reports_the_length<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &[7; 2 * BLOCK]);
    let (again, len) = open(rt, &mut io, "f", true).unwrap();
    assert_eq!(2 * BLOCK as u64, len);
    assert_eq!(
        vec![7; BLOCK],
        read(rt, &mut io, again, BLOCK as u64, BLOCK)
    );
}

fn short_read_past_the_end<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &block(1));
    assert_eq!(block(1), read(rt, &mut io, file, 0, 4 * BLOCK));
    assert_eq!(
        Vec::<u8>::new(),
        read(rt, &mut io, file, 8 * BLOCK as u64, BLOCK)
    );
}

fn allocate_extends_with_zeros<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &block(1));
    let len = 4 * BLOCK as u64;
    assert_eq!(
        Ok(Done::Allocated),
        done(rt, &mut io, Op::Allocate { file, len })
    );
    assert_eq!(len, size(rt, &mut io, file));
    let bytes = read(rt, &mut io, file, 0, 4 * BLOCK);
    assert_eq!(block(1), bytes[..BLOCK]);
    assert!(bytes[BLOCK..].iter().all(|&b| b == 0));
    let len = BLOCK as u64;
    assert_eq!(
        Ok(Done::Allocated),
        done(rt, &mut io, Op::Allocate { file, len })
    );
    assert_eq!(
        4 * BLOCK as u64,
        size(rt, &mut io, file),
        "allocate never shrinks"
    );
}

fn size_counts_completed_writes<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    assert_eq!(0, size(rt, &mut io, file));
    write(rt, &mut io, file, 3 * BLOCK as u64, &block(2));
    assert_eq!(4 * BLOCK as u64, size(rt, &mut io, file));
}

fn missing_files_are_not_found<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    assert_eq!(Err(IoError::NotFound), open(rt, &mut io, "nope", false));
    assert_eq!(Err(IoError::NotFound), open(rt, &mut io, "d/f", true));
    assert_eq!(Err(IoError::NotFound), list(rt, &mut io, "d"));
    let path = "nope".into();
    assert_eq!(
        Err(IoError::NotFound),
        done(rt, &mut io, Op::Remove { path })
    );
    assert_eq!(Err(IoError::NotFound), create_dir(rt, &mut io, "d/e"));
}

fn directories_hold_files<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    assert_eq!(Ok(Done::Created), create_dir(rt, &mut io, "d"));
    assert_eq!(
        Ok(Done::Present),
        create_dir(rt, &mut io, "d"),
        "creation is idempotent"
    );
    assert_eq!(Ok(BTreeSet::new()), list(rt, &mut io, "d"));
    for name in ["d/a", "d/b", "e"] {
        open(rt, &mut io, name, true).unwrap();
    }
    let expect = |names: &[&str]| Ok(names.iter().map(|&n| n.to_owned()).collect());
    assert_eq!(expect(&["d/a", "d/b"]), list(rt, &mut io, "d"));
    assert_eq!(
        expect(&["d", "e"]),
        list(rt, &mut io, ""),
        "a listing holds entries of any kind"
    );
    assert_eq!(Err(IoError::NotADirectory), list(rt, &mut io, "e"));
    assert_eq!(Err(IoError::NotADirectory), list(rt, &mut io, "e/f"));
    assert_eq!(Err(IoError::Exists), create_dir(rt, &mut io, "e"));
    assert_eq!(Err(IoError::NotADirectory), create_dir(rt, &mut io, "e/f"));
    let removed = |rt: &R, io: &mut R::Io, path: &str| {
        let path = path.to_owned();
        done(rt, io, Op::Remove { path })
    };
    let removed_dir = |rt: &R, io: &mut R::Io, path: &str| {
        let path = path.to_owned();
        done(rt, io, Op::RemoveDir { path })
    };
    assert_eq!(Err(IoError::Failed), removed(rt, &mut io, "d"));
    assert_eq!(Err(IoError::NotADirectory), removed_dir(rt, &mut io, "e"));
    assert_eq!(Err(IoError::Failed), removed_dir(rt, &mut io, "d"));
    assert_eq!(Err(IoError::NotFound), removed_dir(rt, &mut io, "absent"));
    assert_eq!(
        Ok(Done::Synced),
        done(rt, &mut io, Op::SyncDir { dir: "d".into() })
    );
    assert_eq!(
        Ok(Done::Synced),
        done(rt, &mut io, Op::SyncDir { dir: "".into() })
    );
}

fn below_a_file_and_absent_directories<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    open(rt, &mut io, "plain", true).unwrap();
    assert_eq!(
        Err(IoError::NotADirectory),
        open(rt, &mut io, "plain/x", false)
    );
    assert_eq!(
        Err(IoError::NotADirectory),
        open(rt, &mut io, "plain/x", true)
    );
    let path = || "plain/x".to_owned();
    for op in [
        Op::CreateNew { path: path() },
        Op::Remove { path: path() },
        Op::RemoveDir { path: path() },
    ] {
        assert_eq!(Err(IoError::NotADirectory), done(rt, &mut io, op));
    }
    let dir = "no-such-dir".into();
    assert_eq!(
        Err(IoError::NotFound),
        done(rt, &mut io, Op::SyncDir { dir })
    );
}

fn rename<R: Runtime>(
    rt: &R,
    io: &mut R::Io,
    from: &str,
    to: &str,
    replace: bool,
) -> Result<Done, IoError> {
    let (from, to) = (from.to_owned(), to.to_owned());
    done(rt, io, Op::Rename { from, to, replace })
}

/// Make `d/f`, force it, rename it into the parent and then force each directory of `synced`.
pub(crate) fn rename_into_the_parent<R: Runtime>(rt: &R, io: &mut R::Io, synced: &[&str]) {
    let force = |io: &mut R::Io, dir: &str| {
        let dir = dir.to_owned();
        assert_eq!(Ok(Done::Synced), done(rt, io, Op::SyncDir { dir }));
    };
    create_dir(rt, io, "d").unwrap();
    open(rt, io, "d/f", true).unwrap();
    force(io, "d");
    force(io, "");
    assert_eq!(Ok(Done::Renamed), rename(rt, io, "d/f", "f", false));
    for dir in synced {
        force(io, dir);
    }
}

/// Check what a crash left of [`rename_into_the_parent`]: the file at its new name when both
/// directories were forced, else at exactly one name. Returns the names.
pub(crate) fn after_a_parent_rename(files: Vec<String>, synced: &[&str], seed: u64) -> Vec<String> {
    if synced.len() == 2 {
        assert_eq!(vec!["f"], files, "seed {seed}: forced");
    } else {
        assert!(
            files == ["f"] || files == ["d/f"],
            "seed {seed} {synced:?}: {files:?}"
        );
    }
    files
}

pub(crate) const PARENT_SYNCS: [&[&str]; 3] = [&["d", ""], &["d"], &[""]];

/// A rename into the parent survives a crash once both directories are forced after it, and else
/// is kept or lost whole.
#[test]
fn a_rename_into_the_parent_is_forced_by_both_directories_on_the_sim() {
    let mut seen = BTreeSet::new();
    for seed in 0..16 {
        for synced in PARENT_SYNCS {
            let mut sim = Sim::new(seed, Config::default());
            adtb_simkit::common::run_then_crash(&mut sim, |rt, io| {
                rename_into_the_parent(rt, io, synced);
            });
            let files = sim.files().into_keys().collect();
            seen.insert(after_a_parent_rename(files, synced, seed));
        }
    }
    assert_eq!(2, seen.len(), "{seen:?}");
}

fn renames_refuse_an_occupied_name<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    for dir in ["d", "d/empty", "d/full", "d/src", "d/src/inner"] {
        create_dir(rt, &mut io, dir).unwrap();
    }
    let (held, _) = open(rt, &mut io, "d/src/inner/f", true).unwrap();
    write(rt, &mut io, held, 0, &block(7));
    for path in ["d/full/g", "d/file", "d/other"] {
        open(rt, &mut io, path, true).unwrap();
    }
    let mut refused = |from, to, error| {
        assert_eq!(
            Err(error),
            rename(rt, &mut io, from, to, false),
            "{from} to {to}"
        );
    };
    refused("d/absent", "d/new", IoError::NotFound);
    refused("d/src", "d/full", IoError::Exists);
    refused("d/src", "d/file", IoError::Exists);
    refused("d/file", "d/other", IoError::Exists);
    refused("d/file", "d/empty", IoError::Exists);
    refused("d/file/a", "d/file/b", IoError::NotADirectory);
    assert_eq!(
        Ok(Done::Renamed),
        rename(rt, &mut io, "d/src", "d/empty", false),
        "onto an empty directory"
    );
    assert_eq!(
        Err(IoError::NotFound),
        open(rt, &mut io, "d/src/inner/f", false)
    );
    let (again, _) = open(rt, &mut io, "d/empty/inner/f", false).unwrap();
    assert_eq!(block(7), read(rt, &mut io, again, 0, BLOCK));
    write(rt, &mut io, held, 0, &block(8));
    assert_eq!(
        block(8),
        read(rt, &mut io, again, 0, BLOCK),
        "an id outlives its name"
    );
    assert_eq!(
        Ok(Done::Renamed),
        rename(rt, &mut io, "d/empty", "d/fresh", false),
        "onto nothing"
    );
    let (other, _) = open(rt, &mut io, "d/other", false).unwrap();
    write(rt, &mut io, other, 0, &block(9));
    assert_eq!(
        Ok(Done::Renamed),
        rename(rt, &mut io, "d/other", "d/file", true),
        "a replacing rename"
    );
    let (file, len) = open(rt, &mut io, "d/file", false).unwrap();
    assert_eq!(BLOCK as u64, len);
    assert_eq!(block(9), read(rt, &mut io, file, 0, BLOCK));
    assert_eq!(Err(IoError::NotFound), open(rt, &mut io, "d/other", false));
    let dir = "d".into();
    assert_eq!(Ok(Done::Synced), done(rt, &mut io, Op::SyncDir { dir }));
}

fn removed_files_are_gone_but_open<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &block(5));
    let path = "f".into();
    assert_eq!(Ok(Done::Removed), done(rt, &mut io, Op::Remove { path }));
    assert_eq!(Err(IoError::NotFound), open(rt, &mut io, "f", false));
    assert_eq!(
        block(5),
        read(rt, &mut io, file, 0, BLOCK),
        "an id outlives its name"
    );
}

fn closed_files_reopen<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &block(6));
    assert_eq!(Ok(Done::Closed), done(rt, &mut io, Op::Close { file }));
    let (again, len) = open(rt, &mut io, "f", false).unwrap();
    assert_eq!(BLOCK as u64, len);
    assert_eq!(block(6), read(rt, &mut io, again, 0, BLOCK));
}

fn every_op_completes_once<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    let mut tickets: BTreeSet<Ticket> = (0..32u8)
        .map(|i| {
            let buf = AlignedBuf::from_slice(&block(i));
            io.submit(Op::Write {
                file,
                offset: u64::from(i) * BLOCK as u64,
                buf,
            })
        })
        .collect();
    assert_eq!(32, tickets.len(), "tickets are distinct");
    while !tickets.is_empty() {
        match io.poll() {
            Some(c) => {
                assert!(tickets.remove(&c.ticket), "{:?} completed twice", c.ticket);
                assert_eq!(Ok(Done::Written), c.result);
                assert_eq!(
                    Some(BLOCK),
                    c.buf.map(|b| b.len()),
                    "a write returns its buffer"
                );
            }
            None => rt.park(),
        }
    }
    for i in 0..32u8 {
        let at = u64::from(i) * BLOCK as u64;
        assert_eq!(block(i), read(rt, &mut io, file, at, BLOCK), "block {i}");
    }
}

fn queues_on_many_threads_complete<R: Runtime>(rt: &R) {
    let workers: Vec<_> = (0..4u8)
        .map(|w| {
            let rt2 = rt.clone();
            rt.spawn("worker", move || {
                let rt = &rt2;
                let mut io = rt.io();
                let (file, _) = open(rt, &mut io, &format!("w{w}"), true).unwrap();
                let tickets: BTreeSet<Ticket> = (0..16u8)
                    .map(|i| {
                        let buf = AlignedBuf::from_slice(&block(w * 16 + i));
                        io.submit(Op::Write {
                            file,
                            offset: u64::from(i) * BLOCK as u64,
                            buf,
                        })
                    })
                    .collect();
                let mut pending = tickets;
                while !pending.is_empty() {
                    match io.poll() {
                        Some(c) => {
                            assert!(pending.remove(&c.ticket), "{:?} completed twice", c.ticket);
                            assert_eq!(Ok(Done::Written), c.result);
                        }
                        None => rt.park(),
                    }
                }
                for i in 0..16u8 {
                    let at = u64::from(i) * BLOCK as u64;
                    assert_eq!(
                        block(w * 16 + i),
                        read(rt, &mut io, file, at, BLOCK),
                        "worker {w} block {i}"
                    );
                }
            })
        })
        .collect();
    for w in workers {
        rt.join(w);
    }
}

fn timer_fires_once_due<R: Runtime>(rt: &R) {
    let mut io = rt.io();
    let start = rt.now();
    let at = start + Duration::from_millis(20);
    let early = rt.now() + Duration::from_millis(1);
    let late = io.submit(Op::Timer { at });
    let first = io.submit(Op::Timer { at: early });
    let mut order = Vec::new();
    while order.len() < 2 {
        match io.poll() {
            Some(c) => {
                assert_eq!(Ok(Done::Fired), c.result);
                assert!(rt.now() >= if c.ticket == late { at } else { early });
                order.push(c.ticket);
            }
            None => rt.park(),
        }
    }
    assert_eq!(vec![first, late], order);
}

fn clock_never_goes_backwards<R: Runtime>(rt: &R) {
    let mut last = rt.now();
    for _ in 0..1000 {
        let now = rt.now();
        assert!(now >= last);
        last = now;
    }
}

fn entropy_varies<R: Runtime>(rt: &R) {
    let draws: BTreeSet<u64> = (0..16).map(|_| rt.next_u64()).collect();
    assert!(draws.len() > 1);
    let mut bytes = [0; 37];
    rt.fill(&mut bytes);
    assert!(bytes.iter().any(|&b| b != 0));
}

fn threads_park_join_and_keep_tokens<R: Runtime>(rt: &R) {
    let me = rt.current();
    rt.unpark(me);
    rt.park();
    let flag = Arc::new(AtomicBool::new(false));
    let child = {
        let rt2 = rt.clone();
        let flag = Arc::clone(&flag);
        rt.spawn("child", move || {
            flag.store(true, Ordering::SeqCst);
            rt2.unpark(me);
        })
    };
    while !flag.load(Ordering::SeqCst) {
        rt.park();
    }
    rt.join(child);
    assert_ne!(me, child);
}

fn unpark_after_exit_does_nothing<R: Runtime>(rt: &R) {
    let child = rt.spawn("child", || {});
    rt.join(child);
    rt.unpark(child);
    let me = rt.current();
    rt.unpark(me);
    rt.park();
}

fn on_sim(body: fn(&SimRuntime)) {
    for seed in 0..4 {
        let mut sim = Sim::new(seed, Config::default());
        assert_eq!(Outcome::Finished, sim.run(|rt| body(&rt)), "seed {seed}");
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("io_contract")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn on_pool(name: &str, body: fn(&OsRuntime)) {
    let rt = OsRuntime::new(scratch(name), Backend::Pool { threads: 4 });
    body(&rt);
}

fn on_uring(name: &str, body: fn(&OsRuntime)) {
    if let Some(backend) = adtb_simkit::backends::uring(4) {
        body(&OsRuntime::new(scratch(name), backend));
    }
}

fn on_faulty(name: &str, backend: Option<Backend>, body: fn(&OsRuntime)) {
    if let Some(backend) = backend {
        body(&OsRuntime::with_faults(
            scratch(name),
            backend,
            Faults::none(4096),
        ));
    }
}

macro_rules! contract {
    ($($name:ident),* $(,)?) => {
        mod sim {
            $( #[test] fn $name() { super::on_sim(super::$name::<adtb_sim::SimRuntime>) } )*
        }
        mod pool {
            $(
                #[test]
                fn $name() {
                    super::on_pool(
                        concat!("pool-", stringify!($name)),
                        super::$name::<adtb_sys::os::OsRuntime>,
                    )
                }
            )*
        }
        mod uring {
            $(
                #[test]
                fn $name() {
                    super::on_uring(
                        concat!("uring-", stringify!($name)),
                        super::$name::<adtb_sys::os::OsRuntime>,
                    )
                }
            )*
        }
        mod faulty_pool {
            $(
                #[test]
                fn $name() {
                    super::on_faulty(
                        concat!("faulty-", stringify!($name)),
                        Some(adtb_sys::os::Backend::Pool { threads: 4 }),
                        super::$name::<adtb_sys::os::OsRuntime>,
                    )
                }
            )*
        }
        mod faulty_uring {
            $(
                #[test]
                fn $name() {
                    super::on_faulty(
                        concat!("faulty-uring-", stringify!($name)),
                        adtb_simkit::backends::uring(4),
                        super::$name::<adtb_sys::os::OsRuntime>,
                    )
                }
            )*
        }
    };
}

contract!(
    write_then_read,
    open_reports_the_length,
    short_read_past_the_end,
    allocate_extends_with_zeros,
    size_counts_completed_writes,
    missing_files_are_not_found,
    directories_hold_files,
    removed_files_are_gone_but_open,
    below_a_file_and_absent_directories,
    renames_refuse_an_occupied_name,
    closed_files_reopen,
    every_op_completes_once,
    queues_on_many_threads_complete,
    timer_fires_once_due,
    clock_never_goes_backwards,
    entropy_varies,
    threads_park_join_and_keep_tokens,
    unpark_after_exit_does_nothing,
);

fn unaligned_requests_fail(rt: &OsRuntime) {
    let mut io = rt.io();
    let (file, _) = open(rt, &mut io, "f", true).unwrap();
    write(rt, &mut io, file, 0, &block(1));
    let unaligned = [
        Op::Write {
            file,
            offset: 1,
            buf: AlignedBuf::from_slice(&block(2)),
        },
        Op::Write {
            file,
            offset: 0,
            buf: AlignedBuf::from_slice(&[3; 100]),
        },
        Op::Read {
            file,
            offset: 1,
            buf: AlignedBuf::zeroed(BLOCK),
        },
        Op::Read {
            file,
            offset: 0,
            buf: AlignedBuf::zeroed(100),
        },
    ];
    for op in unaligned {
        let shown = format!("{op:?}");
        let c = complete(rt, &mut io, op);
        assert_eq!(Err(IoError::Failed), c.result, "{shown}");
        assert!(c.buf.is_some(), "a failed {shown} returns its buffer");
    }
    assert_eq!(block(1), read(rt, &mut io, file, 0, BLOCK));
}

#[test]
fn unaligned_requests_fail_on_the_pool() {
    on_pool("pool-unaligned", unaligned_requests_fail);
}

#[test]
fn unaligned_requests_fail_on_the_uring() {
    on_uring("uring-unaligned", unaligned_requests_fail);
}

#[test]
fn a_refused_ring_fails_the_test_unless_its_absence_is_declared() {
    let absent = std::env::var("ADTB_URING").as_deref() == Ok("absent");
    let chosen = std::panic::catch_unwind(|| adtb_simkit::backends::uring(u32::MAX));
    match chosen {
        Ok(backend) => {
            assert!(absent, "a refused ring is not chosen");
            assert_eq!(None, backend);
        }
        Err(payload) => {
            assert!(!absent, "a declared absence does not panic");
            let message = payload.downcast_ref::<String>().expect("a formatted panic");
            assert!(
                message.ends_with("; set ADTB_URING=absent on a host without io_uring"),
                "{message}"
            );
        }
    }
}

const SUBMITTED: u8 = 64;

/// Submit writes without waiting for them, then drop the runtime.
fn writes_submitted_before_a_drop_have_landed_when_it_returns(name: &str, backend: Backend) {
    let dir = scratch(name);
    let rt = OsRuntime::new(&dir, backend);
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    for i in 0..SUBMITTED {
        let buf = AlignedBuf::from_slice(&block(i + 1));
        io.submit(Op::Write {
            file,
            offset: u64::from(i) * BLOCK as u64,
            buf,
        });
    }
    drop(io);
    drop(rt);
    let bytes = std::fs::read(dir.join("f")).expect("the file");
    let landed: Vec<u8> = bytes.chunks(BLOCK).map(|b| b[0]).collect();
    assert_eq!((1..=SUBMITTED).collect::<Vec<u8>>(), landed);
}

#[test]
fn writes_submitted_before_a_drop_have_landed_when_it_returns_on_the_pool() {
    writes_submitted_before_a_drop_have_landed_when_it_returns(
        "pool-drop-drains",
        Backend::Pool { threads: 4 },
    );
}

#[test]
fn writes_submitted_before_a_drop_have_landed_when_it_returns_on_the_uring() {
    if let Some(backend) = adtb_simkit::backends::uring(4) {
        writes_submitted_before_a_drop_have_landed_when_it_returns("uring-drop-drains", backend);
    }
}
