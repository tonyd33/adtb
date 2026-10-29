use adtb_simkit::common;

use std::collections::BTreeSet;
use std::time::Duration;

use adtb::runtime::{AlignedBuf, Done, Io, IoError, Op, Runtime, Threads};
use adtb_sim::{Config, Faults, Outcome, Sim, SimIo, SimRuntime};
use common::{SECTOR, complete, open, read, run, run_then_crash, sector, sync, sync_dir, write};

fn quiet() -> Faults {
    Faults {
        delay_completions: false,
        ..Faults::default()
    }
}

fn sim(seed: u64, faults: Faults) -> Sim {
    Sim::new(
        seed,
        Config {
            faults,
            ..Config::default()
        },
    )
}

/// Create file `f` holding `sectors` sectors filled 1, 2, 3, ..., forced with its name.
fn setup(sim: &mut Sim, sectors: u8) {
    run(sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        for i in 0..sectors {
            write(rt, io, file, u64::from(i) * SECTOR as u64, &sector(i + 1)).unwrap();
        }
        sync(rt, io, file).unwrap();
        sync_dir(rt, io, "").unwrap();
    });
}

fn create_dir(rt: &SimRuntime, io: &mut SimIo, dir: &str) {
    assert_eq!(
        Ok(Done::Created),
        complete(rt, io, Op::CreateDir { dir: dir.into() }).result
    );
}

fn read_all(sim: &mut Sim) -> Vec<u8> {
    run(sim, |rt, io| {
        let (file, len) = open(rt, io, "f", false).unwrap();
        read(rt, io, file, 0, len as usize).unwrap()
    })
}

#[test]
fn torn_write_lands_some_atomic_units() {
    let mut mixed = false;
    for seed in 0..32 {
        let mut sim = sim(
            seed,
            Faults {
                torn_p: 1.0,
                ..quiet()
            },
        );
        setup(&mut sim, 8);
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", false).unwrap();
            write(rt, io, file, 0, &[0xbb; 8 * SECTOR]).unwrap();
        });
        let bytes = read_all(&mut sim);
        let mut new = 0;
        for (i, s) in bytes.chunks(SECTOR).enumerate() {
            let old = sector(i as u8 + 1);
            assert!(
                s == old || s == [0xbb; SECTOR],
                "seed {seed}: sector {i} is neither old nor new"
            );
            new += usize::from(s[0] == 0xbb);
        }
        mixed |= new > 0 && new < 8;
    }
    assert!(mixed, "no seed tore the write");
}

#[test]
fn write_within_atomic_unit_never_tears() {
    for seed in 0..32 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    torn_p: 1.0,
                    ..quiet()
                },
                atomic_unit: 4096,
                ..Config::default()
            },
        );
        setup(&mut sim, 8);
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", false).unwrap();
            write(rt, io, file, 0, &[0xbb; 8 * SECTOR]).unwrap();
        });
        let bytes = read_all(&mut sim);
        let new = bytes.chunks(SECTOR).filter(|s| s[0] == 0xbb).count();
        assert!(
            new == 0 || new == 8,
            "seed {seed} tore an atomic write into {new} of 8 sectors"
        );
    }
}

#[test]
fn bit_rot_flips_one_stored_bit() {
    let mut sim = sim(1, quiet());
    setup(&mut sim, 1);
    sim.set_faults(Faults {
        corrupt_p: 1.0,
        ..quiet()
    });
    let rotted = read_all(&mut sim);
    let flipped: u32 = rotted
        .iter()
        .zip(sector(1))
        .map(|(a, b)| (a ^ b).count_ones())
        .sum();
    assert_eq!(1, flipped);
    sim.set_faults(quiet());
    assert_eq!(
        rotted,
        read_all(&mut sim),
        "rot is on the device, not in one read"
    );
}

#[test]
fn misdirected_write_lands_elsewhere() {
    let mut sim = sim(2, quiet());
    setup(&mut sim, 4);
    sim.set_faults(Faults {
        misdirect_p: 1.0,
        ..quiet()
    });
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        write(rt, io, file, SECTOR as u64, &sector(0xee)).unwrap();
    });
    sim.set_faults(quiet());
    let bytes = read_all(&mut sim);
    let sectors: Vec<&[u8]> = bytes.chunks(SECTOR).collect();
    assert_eq!(sector(2), sectors[1], "the target keeps its old contents");
    assert_eq!(1, sectors.iter().filter(|s| s[0] == 0xee).count());
}

#[test]
fn lost_write_is_acknowledged_and_absent() {
    let mut sim = sim(3, quiet());
    setup(&mut sim, 1);
    sim.set_faults(Faults {
        lost_write_p: 1.0,
        ..quiet()
    });
    let seen = run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        write(rt, io, file, 0, &sector(0xee)).unwrap();
        sync(rt, io, file).unwrap();
        read(rt, io, file, 0, SECTOR).unwrap()
    });
    assert_eq!(sector(1), seen);
    sim.set_faults(quiet());
    run_then_crash(&mut sim, |_, _| {});
    assert_eq!(sector(1), read_all(&mut sim));
}

#[test]
fn latent_sector_error_persists_until_rewritten() {
    let mut sim = sim(4, quiet());
    setup(&mut sim, 1);
    sim.set_faults(Faults {
        latent_p: 1.0,
        ..quiet()
    });
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        assert_eq!(Err(IoError::Failed), read(rt, io, file, 0, SECTOR));
    });
    sim.set_faults(quiet());
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        assert_eq!(Err(IoError::Failed), read(rt, io, file, 0, SECTOR));
        write(rt, io, file, 0, &sector(9)).unwrap();
        assert_eq!(Ok(sector(9)), read(rt, io, file, 0, SECTOR));
    });
}

#[test]
fn a_device_with_no_space_refuses_growth_and_keeps_overwrites() {
    let mut sim = sim(9, quiet());
    setup(&mut sim, 1);
    sim.set_faults(Faults {
        no_space: true,
        ..quiet()
    });
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        assert_eq!(
            Err(IoError::Full),
            write(rt, io, file, SECTOR as u64, &sector(2))
        );
        let grow = Op::Allocate {
            file,
            len: 2 * SECTOR as u64,
        };
        assert_eq!(Err(IoError::Full), complete(rt, io, grow).result);
        write(rt, io, file, 0, &sector(7)).unwrap();
        assert_eq!(Ok(sector(7)), read(rt, io, file, 0, 2 * SECTOR));
    });
}

#[test]
fn write_and_force_fail() {
    let mut sim = sim(5, quiet());
    setup(&mut sim, 1);
    sim.set_faults(Faults {
        fail_p: 1.0,
        ..quiet()
    });
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        assert_eq!(Err(IoError::Failed), write(rt, io, file, 0, &sector(9)));
        assert_eq!(Err(IoError::Failed), sync(rt, io, file));
        assert_eq!(Err(IoError::Failed), sync_dir(rt, io, ""));
    });
}

#[test]
fn crash_keeps_forced_writes_and_loses_some_unforced() {
    let (mut kept, mut lost) = (false, false);
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        setup(&mut sim, 2);
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", false).unwrap();
            write(rt, io, file, 0, &sector(0xaa)).unwrap();
            sync(rt, io, file).unwrap();
            write(rt, io, file, SECTOR as u64, &sector(0xbb)).unwrap();
        });
        let bytes = read_all(&mut sim);
        assert_eq!(
            sector(0xaa),
            bytes[..SECTOR],
            "seed {seed} lost a forced write"
        );
        if bytes[SECTOR..] == sector(0xbb)[..] {
            kept = true;
        } else {
            assert_eq!(sector(2), bytes[SECTOR..], "seed {seed}");
            lost = true;
        }
    }
    assert!(
        kept && lost,
        "unforced writes should sometimes survive and sometimes not"
    );
}

#[test]
fn without_lose_unsynced_completed_writes_survive_a_crash() {
    for seed in 0..16 {
        let mut sim = sim(
            seed,
            Faults {
                lose_unsynced: false,
                ..quiet()
            },
        );
        setup(&mut sim, 1);
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", false).unwrap();
            write(rt, io, file, 0, &sector(0xbb)).unwrap();
        });
        assert_eq!(sector(0xbb), read_all(&mut sim), "seed {seed}");
    }
}

#[test]
fn write_in_flight_at_a_crash_may_land() {
    let (mut kept, mut lost) = (false, false);
    for seed in 0..32 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    lose_unsynced: false,
                    ..Faults::default()
                },
                latency: (Duration::from_secs(5), Duration::from_secs(5)),
                ..Config::default()
            },
        );
        setup(&mut sim, 1);
        sim.crash_at(sim.now() + Duration::from_secs(7));
        let outcome = sim.run(|rt| {
            let mut io = rt.io();
            let (file, _) = open(&rt, &mut io, "f", false).unwrap();
            io.submit(Op::Write {
                file,
                offset: 0,
                buf: AlignedBuf::from_slice(&sector(0xbb)),
            });
            rt.park();
            unreachable!("the crash comes before the completion");
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        let bytes = sim.files()["f"].clone();
        if bytes == sector(0xbb) {
            kept = true;
        } else {
            assert_eq!(sector(1), bytes, "seed {seed}");
            lost = true;
        }
    }
    assert!(kept && lost);
}

#[test]
fn creation_without_directory_force_may_vanish() {
    let mut vanished = false;
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", true).unwrap();
            write(rt, io, file, 0, &sector(1)).unwrap();
            sync(rt, io, file).unwrap();
        });
        vanished |= !sim.files().contains_key("f");
    }
    assert!(vanished);
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        run_then_crash(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", true).unwrap();
            write(rt, io, file, 0, &sector(1)).unwrap();
            sync(rt, io, file).unwrap();
            sync_dir(rt, io, "").unwrap();
        });
        assert_eq!(Some(&sector(1)), sim.files().get("f"), "seed {seed}");
    }
}

#[test]
fn directory_force_covers_only_its_directory() {
    let mut vanished = false;
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        run_then_crash(&mut sim, |rt, io| {
            create_dir(rt, io, "log");
            open(rt, io, "log/0", true).unwrap();
            open(rt, io, "data", true).unwrap();
            sync_dir(rt, io, "").unwrap();
        });
        assert!(sim.files().contains_key("data"), "seed {seed}");
        vanished |= !sim.files().contains_key("log/0");
    }
    assert!(vanished);
}

#[test]
fn unforced_directory_takes_its_files_at_a_crash() {
    let mut vanished = false;
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        run_then_crash(&mut sim, |rt, io| {
            create_dir(rt, io, "log");
            open(rt, io, "log/0", true).unwrap();
            sync_dir(rt, io, "log").unwrap();
        });
        let files = sim.files();
        vanished |= !files.contains_key("log/0");
        let survived = run(&mut sim, |rt, io| {
            complete(rt, io, Op::List { dir: "log".into() }).result
        });
        assert_eq!(
            files.contains_key("log/0"),
            survived.is_ok(),
            "seed {seed}: the file outlives its directory or the reverse"
        );
    }
    assert!(vanished);
}

#[test]
fn directory_force_keeps_the_directory_and_its_files() {
    for seed in 0..16 {
        let mut sim = sim(seed, quiet());
        run_then_crash(&mut sim, |rt, io| {
            create_dir(rt, io, "log");
            sync_dir(rt, io, "").unwrap();
            open(rt, io, "log/0", true).unwrap();
            sync_dir(rt, io, "log").unwrap();
        });
        assert!(sim.files().contains_key("log/0"), "seed {seed}");
    }
}

#[test]
fn removal_needs_a_directory_force() {
    let mut back = false;
    for seed in 0..32 {
        let mut sim = sim(seed, quiet());
        setup(&mut sim, 1);
        run_then_crash(&mut sim, |rt, io| {
            assert_eq!(
                Ok(Done::Removed),
                complete(rt, io, Op::Remove { path: "f".into() }).result,
                "seed {seed}"
            );
            assert_eq!(
                Err(IoError::NotFound),
                open(rt, io, "f", false).map(|_| ()),
                "seed {seed}"
            );
        });
        back |= sim.files().contains_key("f");
    }
    assert!(back);
}

#[test]
fn delayed_completions_come_out_of_order() {
    let order = |seed, faults| {
        let mut sim = sim(seed, faults);
        setup(&mut sim, 8);
        run(&mut sim, |rt, io| {
            let (file, _) = open(rt, io, "f", false).unwrap();
            let tickets: Vec<_> = (0..8)
                .map(|i| {
                    io.submit(Op::Read {
                        file,
                        offset: i * SECTOR as u64,
                        buf: AlignedBuf::zeroed(SECTOR),
                    })
                })
                .collect();
            let mut seen = Vec::new();
            while seen.len() < 8 {
                match io.poll() {
                    Some(c) => seen.push(tickets.iter().position(|&t| t == c.ticket).unwrap()),
                    None => rt.park(),
                }
            }
            seen
        })
    };
    let in_order: Vec<usize> = (0..8).collect();
    assert!((0..8).any(|seed| order(seed, Faults::default()) != in_order));
    for seed in 0..8 {
        assert_eq!(in_order, order(seed, quiet()), "seed {seed}");
    }
}

#[test]
fn listing_order_varies() {
    let list = |seed| {
        let mut sim = sim(seed, quiet());
        run(&mut sim, |rt, io| {
            create_dir(rt, io, "d");
            for name in ["d/a", "d/b", "d/c", "d/d", "e"] {
                open(rt, io, name, true).unwrap();
            }
            match complete(rt, io, Op::List { dir: "d".into() }).result {
                Ok(Done::Listed(names)) => names,
                other => panic!("{other:?}"),
            }
        })
    };
    let orders: BTreeSet<Vec<String>> = (0..16).map(list).collect();
    assert!(orders.len() > 1);
    for names in orders {
        let set: BTreeSet<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(BTreeSet::from(["d/a", "d/b", "d/c", "d/d"]), set);
    }
}

#[test]
fn short_read_past_the_end() {
    let mut sim = sim(6, quiet());
    setup(&mut sim, 1);
    run(&mut sim, |rt, io| {
        let (file, len) = open(rt, io, "f", false).unwrap();
        assert_eq!(SECTOR as u64, len);
        assert_eq!(Ok(sector(1)), read(rt, io, file, 0, 4 * SECTOR));
        assert_eq!(Ok(vec![]), read(rt, io, file, 8 * SECTOR as u64, SECTOR));
    });
}

#[test]
fn allocate_extends_with_zeros() {
    let mut sim = sim(7, quiet());
    setup(&mut sim, 1);
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", false).unwrap();
        assert_eq!(
            Ok(Done::Allocated),
            complete(
                rt,
                io,
                Op::Allocate {
                    file,
                    len: 4 * SECTOR as u64
                }
            )
            .result
        );
        let bytes = read(rt, io, file, 0, 4 * SECTOR).unwrap();
        assert_eq!(sector(1), bytes[..SECTOR]);
        assert!(bytes[SECTOR..].iter().all(|&b| b == 0));
    });
}

#[test]
#[should_panic(expected = "not sector aligned")]
fn unaligned_write_is_a_bug() {
    let mut sim = sim(8, quiet());
    run(&mut sim, |rt, io| {
        let (file, _) = open(rt, io, "f", true).unwrap();
        write(rt, io, file, 100, &sector(1)).unwrap();
    });
}

#[test]
fn open_missing_file_is_not_found() {
    let mut sim = sim(9, quiet());
    run(&mut sim, |rt, io| {
        assert_eq!(
            Err(IoError::NotFound),
            open(rt, io, "nope", false).map(|_| ())
        );
    });
}
