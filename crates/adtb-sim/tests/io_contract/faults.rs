//! The faults the real runtime injects beneath its files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use adtb::runtime::{AlignedBuf, Done, IoError, Op, Runtime};
use adtb_sys::os::{Backend, Faults, OsIo, OsRuntime};

use super::{BLOCK, block, complete, done, list, open, read, scratch, write};

const SEEDS: u64 = 24;

/// The backend a case runs on, and the name its directories begin with.
#[derive(Clone, Copy)]
struct On(&'static str, Backend);

impl On {
    fn scratch(self, name: &str) -> PathBuf {
        scratch(&format!("{}-{name}", self.0))
    }

    fn faulty(self, dir: &Path, faults: Faults) -> OsRuntime {
        OsRuntime::with_faults(dir, self.1, faults)
    }
}

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        mod pool {
            $(
                #[test]
                fn $name() {
                    super::$name(super::On("pool", adtb_sys::os::Backend::Pool { threads: 2 }));
                }
            )*
        }
        mod uring {
            $(
                #[test]
                fn $name() {
                    if let Some(backend) = adtb_simkit::backends::uring(4) {
                        super::$name(super::On("uring", backend));
                    }
                }
            )*
        }
    };
}

cases!(
    failed_writes_allocations_and_forces_complete_with_failed,
    a_lost_write_completes_and_never_lands,
    a_misdirected_write_lands_at_another_offset_of_the_file,
    a_latent_sector_fails_reads_until_it_is_rewritten,
    a_rotted_bit_is_seen_by_every_later_read,
    a_failed_force_loses_writes_and_a_later_force_succeeds_without_them,
    a_crash_keeps_forced_writes_and_any_subset_of_the_rest,
    a_torn_write_lands_in_any_subset_of_its_atomic_units,
    a_crash_keeps_a_prefix_of_unforced_creations_and_removals,
    a_crash_may_lose_an_unforced_rename_and_removed_directory_whole,
    a_crash_that_loses_a_directory_loses_its_files,
);

fn plain(dir: &Path) -> OsRuntime {
    OsRuntime::new(dir, Backend::Pool { threads: 2 })
}

fn failing_write(rt: &OsRuntime, io: &mut OsIo, data: &[u8]) {
    let (file, _) = open(rt, io, "f", false).unwrap();
    let buf = AlignedBuf::from_slice(data);
    let c = complete(
        rt,
        io,
        Op::Write {
            file,
            offset: 0,
            buf,
        },
    );
    assert_eq!(Err(IoError::Failed), c.result);
    assert_eq!(
        Some(data),
        c.buf.as_deref(),
        "a failed write returns its buffer"
    );
}

fn failed_writes_allocations_and_forces_complete_with_failed(on: On) {
    let rt = on.faulty(&on.scratch("faults-fail"), Faults::default());
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    rt.set_faults(Faults {
        fail_p: 1.0,
        ..Faults::default()
    });
    failing_write(&rt, &mut io, &block(1));
    let fails = [
        Op::Allocate {
            file,
            len: 4 * BLOCK as u64,
        },
        Op::Sync { file },
        Op::SyncDir { dir: String::new() },
    ];
    for op in fails {
        assert_eq!(Err(IoError::Failed), done(&rt, &mut io, op));
    }
}

fn a_lost_write_completes_and_never_lands(on: On) {
    let rt = on.faulty(&on.scratch("faults-lost"), Faults::default());
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    write(&rt, &mut io, file, 0, &block(1));
    rt.set_faults(Faults {
        lost_write_p: 1.0,
        ..Faults::default()
    });
    write(&rt, &mut io, file, 0, &block(2));
    assert_eq!(Ok(Done::Synced), done(&rt, &mut io, Op::Sync { file }));
    rt.set_faults(Faults::default());
    assert_eq!(block(1), read(&rt, &mut io, file, 0, BLOCK));
}

fn a_misdirected_write_lands_at_another_offset_of_the_file(on: On) {
    let rt = on.faulty(&on.scratch("faults-misdirect"), Faults::default());
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    for i in 0..4u8 {
        write(
            &rt,
            &mut io,
            file,
            u64::from(i) * BLOCK as u64,
            &block(i + 1),
        );
    }
    rt.set_faults(Faults {
        misdirect_p: 1.0,
        ..Faults::default()
    });
    write(&rt, &mut io, file, BLOCK as u64, &block(9));
    rt.set_faults(Faults::default());
    let bytes = read(&rt, &mut io, file, 0, 4 * BLOCK);
    let blocks: Vec<&[u8]> = bytes.chunks(BLOCK).collect();
    assert_eq!(block(2), blocks[1], "the target keeps its old contents");
    assert_eq!(
        1,
        blocks.iter().filter(|b| **b == block(9)).count(),
        "the data lands at one other offset"
    );
}

fn a_latent_sector_fails_reads_until_it_is_rewritten(on: On) {
    let rt = on.faulty(&on.scratch("faults-latent"), Faults::default());
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    write(&rt, &mut io, file, 0, &block(1));
    write(&rt, &mut io, file, BLOCK as u64, &block(2));
    rt.set_faults(Faults {
        latent_p: 1.0,
        ..Faults::default()
    });
    let read_first = |io: &mut OsIo| {
        let buf = AlignedBuf::zeroed(BLOCK);
        let c = complete(
            &rt,
            io,
            Op::Read {
                file,
                offset: 0,
                buf,
            },
        );
        assert!(c.buf.is_some(), "a failed read returns its buffer");
        c.result
    };
    assert_eq!(Err(IoError::Failed), read_first(&mut io));
    rt.set_faults(Faults::default());
    assert_eq!(Err(IoError::Failed), read_first(&mut io));
    assert_eq!(block(2), read(&rt, &mut io, file, BLOCK as u64, BLOCK));
    write(&rt, &mut io, file, 0, &block(3));
    assert_eq!(block(3), read(&rt, &mut io, file, 0, BLOCK));
}

fn a_rotted_bit_is_seen_by_every_later_read(on: On) {
    let rt = on.faulty(&on.scratch("faults-rot"), Faults::default());
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    write(&rt, &mut io, file, 0, &block(0x55));
    rt.set_faults(Faults {
        corrupt_p: 1.0,
        ..Faults::default()
    });
    let rotted = read(&rt, &mut io, file, 0, BLOCK);
    rt.set_faults(Faults::default());
    let flipped: u32 = rotted
        .iter()
        .zip(block(0x55))
        .map(|(a, b)| (a ^ b).count_ones())
        .sum();
    assert_eq!(1, flipped);
    assert_eq!(rotted, read(&rt, &mut io, file, 0, BLOCK));
}

/// Returns a forced file `f` holding `block(1)` at its start, under a fresh directory.
fn forced_file(on: On, name: &str, seed: u64, faults: Faults) -> (OsRuntime, PathBuf) {
    let dir = on.scratch(&format!("{name}-{seed}"));
    let rt = on.faulty(&dir, Faults { seed, ..faults });
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", true).unwrap();
    write(&rt, &mut io, file, 0, &block(1));
    assert_eq!(Ok(Done::Synced), done(&rt, &mut io, Op::Sync { file }));
    let root = Op::SyncDir { dir: String::new() };
    assert_eq!(Ok(Done::Synced), done(&rt, &mut io, root));
    (rt, dir)
}

fn read_back(dir: &Path, len: usize) -> Vec<u8> {
    let rt = plain(dir);
    let mut io = rt.io();
    let (file, _) = open(&rt, &mut io, "f", false).unwrap();
    read(&rt, &mut io, file, 0, len)
}

fn a_failed_force_loses_writes_and_a_later_force_succeeds_without_them(on: On) {
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let (rt, dir) = forced_file(on, "faults-fsyncgate", seed, Faults::default());
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", false).unwrap();
        write(&rt, &mut io, file, 0, &block(2));
        rt.set_faults(Faults {
            seed,
            fail_p: 1.0,
            ..Faults::default()
        });
        let sync = |io: &mut OsIo| done(&rt, io, Op::Sync { file });
        assert_eq!(Err(IoError::Failed), sync(&mut io), "seed {seed}");
        rt.set_faults(Faults {
            seed,
            ..Faults::default()
        });
        let after = read(&rt, &mut io, file, 0, BLOCK);
        assert!(
            after == block(1) || after == block(2),
            "seed {seed}: a lost write leaves the forced contents"
        );
        assert_eq!(Ok(Done::Synced), sync(&mut io), "seed {seed}");
        rt.crash();
        assert_eq!(after, read_back(&dir, BLOCK), "seed {seed}");
        seen.insert(after == block(2));
    }
    assert_eq!(BTreeSet::from([false, true]), seen);
}

fn a_crash_keeps_forced_writes_and_any_subset_of_the_rest(on: On) {
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let (rt, dir) = forced_file(on, "faults-crash", seed, Faults::default());
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", false).unwrap();
        write(&rt, &mut io, file, BLOCK as u64, &block(2));
        write(&rt, &mut io, file, 2 * BLOCK as u64, &block(3));
        rt.crash();
        let buf = AlignedBuf::zeroed(BLOCK);
        let after = done(
            &rt,
            &mut io,
            Op::Read {
                file,
                offset: 0,
                buf,
            },
        );
        assert_eq!(
            Err(IoError::Failed),
            after,
            "seed {seed}: an op after a crash"
        );
        let bytes = read_back(&dir, 3 * BLOCK);
        assert_eq!(block(1), bytes[..BLOCK], "seed {seed}");
        let kept: Vec<bool> = [2u8, 3]
            .iter()
            .zip(bytes[BLOCK..].chunks(BLOCK))
            .map(|(&fill, b)| {
                assert!(b == block(fill) || b == block(0), "seed {seed}: {b:?}");
                b == block(fill)
            })
            .collect();
        seen.insert(kept);
    }
    assert_eq!(4, seen.len(), "every subset of two writes: {seen:?}");
}

fn a_torn_write_lands_in_any_subset_of_its_atomic_units(on: On) {
    let mut mixed = false;
    for seed in 0..SEEDS {
        let faults = Faults {
            torn_p: 1.0,
            ..Faults::default()
        };
        let (rt, dir) = forced_file(on, "faults-torn", seed, faults);
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "f", false).unwrap();
        let whole: Vec<u8> = (2..6u8).flat_map(block).collect();
        write(&rt, &mut io, file, 0, &whole);
        rt.crash();
        let bytes = read_back(&dir, 4 * BLOCK);
        let old = [1u8, 0, 0, 0];
        let landed: Vec<bool> = bytes
            .chunks(BLOCK)
            .enumerate()
            .map(|(i, b)| {
                let new = block(i as u8 + 2);
                assert!(b == new || b == block(old[i]), "seed {seed}: unit {i}");
                b == new
            })
            .collect();
        mixed |= landed.contains(&true) && landed.contains(&false);
    }
    assert!(mixed, "some write tears");
}

fn a_crash_keeps_a_prefix_of_unforced_creations_and_removals(on: On) {
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let dir = on.scratch(&format!("faults-names-{seed}"));
        let rt = on.faulty(
            &dir,
            Faults {
                seed,
                ..Faults::default()
            },
        );
        let mut io = rt.io();
        assert_eq!(
            Ok(Done::Created),
            super::create_dir(&rt, &mut io, "d"),
            "seed {seed}"
        );
        let root = Op::SyncDir { dir: String::new() };
        assert_eq!(Ok(Done::Synced), done(&rt, &mut io, root), "seed {seed}");
        open(&rt, &mut io, "d/kept", true).unwrap();
        let d = Op::SyncDir { dir: "d".into() };
        assert_eq!(Ok(Done::Synced), done(&rt, &mut io, d), "seed {seed}");
        open(&rt, &mut io, "d/a", true).unwrap();
        let remove = Op::Remove {
            path: "d/kept".into(),
        };
        assert_eq!(Ok(Done::Removed), done(&rt, &mut io, remove), "seed {seed}");
        open(&rt, &mut io, "d/b", true).unwrap();
        rt.crash();
        let rt = plain(&dir);
        let mut io = rt.io();
        let names: Vec<String> = list(&rt, &mut io, "d")
            .unwrap()
            .into_iter()
            .map(|p| p.trim_start_matches("d/").to_owned())
            .collect();
        let prefixes = [vec!["kept"], vec!["a", "kept"], vec!["a"], vec!["a", "b"]];
        assert!(
            prefixes.iter().any(|p| *p == names),
            "seed {seed}: {names:?} is no prefix"
        );
        seen.insert(names);
    }
    assert_eq!(4, seen.len(), "every prefix: {seen:?}");
}

fn a_crash_may_lose_an_unforced_rename_and_removed_directory_whole(on: On) {
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let dir = on.scratch(&format!("faults-rename-{seed}"));
        let rt = on.faulty(
            &dir,
            Faults {
                seed,
                ..Faults::default()
            },
        );
        let mut io = rt.io();
        for name in ["from", "gone"] {
            super::create_dir(&rt, &mut io, name).unwrap();
        }
        open(&rt, &mut io, "from/f", true).unwrap();
        let force = |d: &str| Op::SyncDir { dir: d.to_owned() };
        for d in ["from", ""] {
            assert_eq!(
                Ok(Done::Synced),
                done(&rt, &mut io, force(d)),
                "seed {seed}"
            );
        }
        let remove = Op::RemoveDir {
            path: "gone".into(),
        };
        assert_eq!(Ok(Done::Removed), done(&rt, &mut io, remove), "seed {seed}");
        let rename = Op::Rename {
            from: "from".into(),
            to: "to".into(),
            replace: false,
        };
        assert_eq!(Ok(Done::Renamed), done(&rt, &mut io, rename), "seed {seed}");
        rt.crash();
        let rt = plain(&dir);
        let mut io = rt.io();
        let names = list(&rt, &mut io, "").unwrap();
        let expect = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<BTreeSet<_>>();
        assert!(
            names == expect(&["from", "gone"])
                || names == expect(&["from"])
                || names == expect(&["to"]),
            "seed {seed}: {names:?}"
        );
        let inner = if names.contains("to") {
            "to/f"
        } else {
            "from/f"
        };
        open(&rt, &mut io, inner, false).unwrap();
        seen.insert(names.into_iter().collect::<Vec<_>>());
    }
    assert_eq!(3, seen.len(), "every prefix: {seen:?}");
}

fn a_crash_that_loses_a_directory_loses_its_files(on: On) {
    let mut lost = false;
    for seed in 0..SEEDS {
        let dir = on.scratch(&format!("faults-dir-{seed}"));
        let rt = on.faulty(
            &dir,
            Faults {
                seed,
                ..Faults::default()
            },
        );
        let mut io = rt.io();
        assert_eq!(
            Ok(Done::Created),
            super::create_dir(&rt, &mut io, "d"),
            "seed {seed}"
        );
        let (file, _) = open(&rt, &mut io, "d/f", true).unwrap();
        write(&rt, &mut io, file, 0, &block(1));
        assert_eq!(
            Ok(Done::Synced),
            done(&rt, &mut io, Op::Sync { file }),
            "seed {seed}"
        );
        let d = Op::SyncDir { dir: "d".into() };
        assert_eq!(Ok(Done::Synced), done(&rt, &mut io, d), "seed {seed}");
        rt.crash();
        let rt = plain(&dir);
        let mut io = rt.io();
        match list(&rt, &mut io, "d") {
            Err(IoError::NotFound) => lost = true,
            Ok(names) => assert_eq!(BTreeSet::from(["d/f".to_owned()]), names, "seed {seed}"),
            Err(e) => panic!("seed {seed}: {e:?}"),
        }
    }
    assert!(lost, "some crash loses the directory");
}
