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
        Ok(Done::Created),
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
        expect(&["e"]),
        list(rt, &mut io, ""),
        "a listing holds only files"
    );
    assert_eq!(
        Ok(Done::Synced),
        done(rt, &mut io, Op::SyncDir { dir: "d".into() })
    );
    assert_eq!(
        Ok(Done::Synced),
        done(rt, &mut io, Op::SyncDir { dir: "".into() })
    );
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

fn on_faulty_pool(name: &str, body: fn(&OsRuntime)) {
    let rt = OsRuntime::with_faults(
        scratch(name),
        Backend::Pool { threads: 4 },
        Faults::default(),
    );
    body(&rt);
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
        mod faulty_pool {
            $(
                #[test]
                fn $name() {
                    super::on_faulty_pool(
                        concat!("faulty-", stringify!($name)),
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
    closed_files_reopen,
    every_op_completes_once,
    timer_fires_once_due,
    clock_never_goes_backwards,
    entropy_varies,
    threads_park_join_and_keep_tokens,
    unpark_after_exit_does_nothing,
);
