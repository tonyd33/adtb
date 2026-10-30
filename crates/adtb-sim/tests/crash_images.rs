//! What a crash leaves on the device: pinned byte for byte on fixed seeds, and stated for allocate
//! as assertions on the image. A digest also folds the simulator's trace, so a difference means
//! its bytes, its random draws or the wording of a trace line changed.

use adtb_simkit::common;

use std::collections::BTreeMap;
use std::time::Duration;

use adtb::runtime::{AlignedBuf, FileId, Io, Op, Runtime};
use adtb_sim::{Config, Faults, Outcome, Sim, SimIo, SimRuntime};
use common::{FNV_OFFSET, SECTOR, allocate, fnv, open, read, run, sector, sync, sync_dir, write};

const SEEDS: u64 = 4;
const MIB: u64 = 1 << 20;

fn fold_files(mut digest: u64, files: &BTreeMap<String, Vec<u8>>) -> u64 {
    for (path, bytes) in files {
        digest = fnv(digest, path.as_bytes());
        digest = fnv(digest, &(bytes.len() as u64).to_le_bytes());
        digest = fnv(digest, bytes);
    }
    digest
}

/// Grow two files by allocation around ordinary writes, forcing only some of each, and return the
/// number of operations that failed.
fn workload(rt: &SimRuntime, io: &mut SimIo) -> u64 {
    let mut failed = 0;
    let (log, _) = open(rt, io, "log", true).expect("create log");
    let (data, _) = open(rt, io, "data", true).expect("create data");
    failed += u64::from(write(rt, io, data, 0, &sector(1)).is_err());
    failed += u64::from(allocate(rt, io, data, MIB).is_err());
    failed += u64::from(allocate(rt, io, log, MIB).is_err());
    failed += u64::from(sync_dir(rt, io, "").is_err());
    failed += u64::from(write(rt, io, log, 0, &[2; 8 * SECTOR]).is_err());
    failed += u64::from(sync(rt, io, log).is_err());
    failed += u64::from(read(rt, io, log, 0, 64 * SECTOR).is_err());
    failed += u64::from(allocate(rt, io, log, 2 * MIB).is_err());
    failed += u64::from(write(rt, io, log, MIB, &[3; 8 * SECTOR]).is_err());
    failed += u64::from(write(rt, io, data, 4 * SECTOR as u64, &sector(4)).is_err());
    failed += u64::from(sync(rt, io, data).is_err());
    failed += u64::from(allocate(rt, io, data, MIB + 8 * SECTOR as u64).is_err());
    failed += u64::from(allocate(rt, io, data, SECTOR as u64).is_err());
    failed += u64::from(write(rt, io, log, 2 * SECTOR as u64, &sector(5)).is_err());
    failed += u64::from(read(rt, io, log, MIB, 16 * SECTOR).is_err());
    failed
}

/// Run the workload to a crash at `after`, then return a digest of the device, the trace and the
/// failure count.
fn crash_digest(seed: u64, faults: &Faults, after: Duration) -> u64 {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: faults.clone(),
            ..Config::default()
        },
    );
    sim.crash_at(sim.now() + after);
    let mut failed = 0;
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        failed = workload(&rt, &mut io);
        common::sleep(&rt, &mut io, Duration::from_secs(3600));
    });
    assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
    let digest = fnv(FNV_OFFSET, &failed.to_le_bytes());
    let digest = fnv(digest, sim.trace().as_bytes());
    fold_files(digest, &sim.files())
}

/// Returns a digest of the images a crash may leave at each force and at the end of a recording.
fn images_digest(seed: u64) -> u64 {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, workload);
    let recording = sim.recording();
    let mut digest = FNV_OFFSET;
    for at in recording.flushes().into_iter().chain([recording.len()]) {
        for image in recording.images(at, 4, seed + at as u64) {
            digest = fold_files(digest, &image);
        }
    }
    digest
}

fn check(name: &str, expected: [u64; SEEDS as usize], digest: impl Fn(u64) -> u64) {
    let actual: Vec<u64> = (0..SEEDS).map(digest).collect();
    assert_eq!(
        expected.as_slice(),
        actual,
        "{name}: digests of seeds 0..{SEEDS}"
    );
}

fn crash_case(name: &str, faults: &Faults, expected: [u64; SEEDS as usize]) {
    check(name, expected, |seed| {
        crash_digest(seed, faults, Duration::from_secs(10))
    });
}

#[test]
fn a_crash_leaves_the_same_image_with_the_default_faults() {
    crash_case(
        "default",
        &Faults::default(),
        [
            0x6cb0_1231_f54c_200c,
            0x2378_0612_5be5_dab8,
            0xe1ce_f85e_aac3_2457,
            0xe021_69bd_fc2d_bb8a,
        ],
    );
}

#[test]
fn a_crash_leaves_the_same_image_when_every_large_write_tears() {
    let faults = Faults {
        torn_p: 1.0,
        ..Faults::default()
    };
    crash_case(
        "torn",
        &faults,
        [
            0xbf47_f44b_eabb_eb59,
            0xadf9_4477_68c2_cd5d,
            0x47b5_ff96_0cd5_ac63,
            0x7103_5117_0293_17ae,
        ],
    );
}

#[test]
fn a_crash_leaves_the_same_image_when_every_completed_write_survives() {
    let faults = Faults {
        lose_unsynced: false,
        ..Faults::default()
    };
    crash_case(
        "kept",
        &faults,
        [
            0x44b1_0e46_17d3_13cf,
            0x18b9_e011_9a9f_fc59,
            0x5283_4696_0ab3_5d00,
            0xf34f_8ef7_9509_9427,
        ],
    );
}

#[test]
fn a_crash_leaves_the_same_image_after_lost_and_misdirected_writes() {
    let faults = Faults {
        lost_write_p: 0.2,
        misdirect_p: 0.2,
        ..Faults::default()
    };
    crash_case(
        "misplaced",
        &faults,
        [
            0xd933_baf2_14a5_d8f1,
            0xf547_6a1a_d96e_f4ab,
            0x3a81_d27b_318d_2336,
            0x4a6d_8eb7_3656_b561,
        ],
    );
}

#[test]
fn a_crash_leaves_the_same_image_after_failed_writes_and_forces() {
    let faults = Faults {
        fail_p: 0.3,
        ..Faults::default()
    };
    crash_case(
        "failing",
        &faults,
        [
            0x9399_b9c2_77a3_abfe,
            0x0c5a_958a_3b01_405f,
            0x5fbd_3bb0_f78e_aebc,
            0x047f_31ec_3ea3_5cd9,
        ],
    );
}

#[test]
fn the_failing_case_fails_an_allocate() {
    let faults = Faults {
        fail_p: 0.3,
        ..Faults::default()
    };
    let failed = (0..SEEDS).any(|seed| {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: faults.clone(),
                ..Config::default()
            },
        );
        run(&mut sim, workload);
        sim.trace().contains("fault fail allocate")
    });
    assert!(failed, "no allocate failed on seeds 0..{SEEDS}");
}

#[test]
fn a_crash_leaves_the_same_image_after_corruption_and_bad_sectors() {
    let faults = Faults {
        corrupt_p: 0.5,
        latent_p: 0.3,
        torn_p: 0.5,
        ..Faults::default()
    };
    crash_case(
        "rotting",
        &faults,
        [
            0x8c32_e390_47c9_7470,
            0x2f2d_a09f_efc9_06c1,
            0xbded_021a_b0c5_dcb6,
            0xc779_c7c6_94c9_90e4,
        ],
    );
}

#[test]
fn a_crash_leaves_the_same_image_on_a_full_device() {
    let faults = Faults {
        no_space: true,
        ..Faults::default()
    };
    crash_case(
        "full",
        &faults,
        [
            0xc99e_a4f9_7a4d_5d4c,
            0x6f4b_a1a7_b7b7_d25e,
            0xda3b_0e05_09c7_0ed3,
            0xebc1_38e6_c8d0_065c,
        ],
    );
}

#[test]
fn a_crash_tearing_a_whole_log_segment_leaves_the_same_image() {
    let faults = Faults {
        torn_p: 1.0,
        delay_completions: false,
        ..Faults::default()
    };
    check(
        "segment",
        [
            0xe1ee_5854_5a4f_eb95,
            0x881f_c040_2558_e164,
            0x79e9_ca4b_5620_324f,
            0x853f_4152_5890_22f9,
        ],
        |seed| {
            let mut sim = Sim::new(
                seed,
                Config {
                    faults: faults.clone(),
                    ..Config::default()
                },
            );
            sim.crash_at(sim.now() + Duration::from_secs(10));
            let outcome = sim.run(|rt| {
                let mut io = rt.io();
                let (log, _) = open(&rt, &mut io, "log", true).expect("create log");
                allocate(&rt, &mut io, log, 16 * MIB).expect("allocate the segment");
                write(&rt, &mut io, log, 0, &sector(6)).expect("write the head");
                common::sleep(&rt, &mut io, Duration::from_secs(3600));
            });
            assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
            fold_files(fnv(FNV_OFFSET, sim.trace().as_bytes()), &sim.files())
        },
    );
}

#[test]
fn a_recording_yields_the_same_crash_images() {
    check(
        "recording",
        [
            0x95d1_5e8f_a272_efdb,
            0x5bd8_abd9_5e74_2817,
            0x22b9_accd_8bb4_675b,
            0xd153_084c_2493_b6bb,
        ],
        images_digest,
    );
}

const SWEEP: u64 = 32;
const UNIT: usize = 8 * SECTOR;
const OLD: usize = 2 * SECTOR;
/// Three whole atomic units and part of a fourth.
const GROWN: usize = 3 * UNIT + SECTOR;

fn torn() -> Faults {
    Faults {
        torn_p: 1.0,
        ..Faults::default()
    }
}

fn kept() -> Faults {
    Faults {
        lose_unsynced: false,
        ..Faults::default()
    }
}

fn device(seed: u64, faults: Faults) -> Sim {
    Sim::new(
        seed,
        Config {
            faults,
            atomic_unit: UNIT as u64,
            latency: (Duration::from_millis(1), Duration::from_millis(1)),
            jitter: Duration::from_nanos(1),
            ..Config::default()
        },
    )
}

/// Returns file `log` after a crash long after `body` runs on it, a file that starts with `OLD`
/// forced bytes of 1.
fn after_completion(
    seed: u64,
    faults: Faults,
    body: impl FnOnce(&SimRuntime, &mut SimIo, FileId),
) -> Vec<u8> {
    let mut sim = device(seed, faults);
    sim.crash_at(sim.now() + Duration::from_secs(10));
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        let file = forced_head(&rt, &mut io);
        body(&rt, &mut io, file);
        common::sleep(&rt, &mut io, Duration::from_secs(3600));
    });
    assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
    sim.files().remove("log").expect("the log survives")
}

/// Returns file `log` after a crash catches `op` on it in flight, a file that starts with `OLD`
/// forced bytes of 1.
fn during(seed: u64, faults: Faults, op: impl FnOnce(FileId) -> Op) -> Vec<u8> {
    let mut sim = device(seed, faults);
    run(&mut sim, |rt, io| {
        forced_head(rt, io);
    });
    sim.crash_at(sim.now() + Duration::from_micros(1500));
    let outcome = sim.run(|rt| {
        let mut io = rt.io();
        let (file, _) = open(&rt, &mut io, "log", false).expect("open log");
        io.submit(op(file));
        common::sleep(&rt, &mut io, Duration::from_secs(3600));
    });
    assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
    sim.files().remove("log").expect("the log survives")
}

fn forced_head(rt: &SimRuntime, io: &mut SimIo) -> FileId {
    let (file, _) = open(rt, io, "log", true).expect("create log");
    sync_dir(rt, io, "").expect("force the name");
    write(rt, io, file, 0, &[1; OLD]).expect("write the head");
    sync(rt, io, file).expect("force the head");
    file
}

#[test]
fn a_forced_allocate_survives_a_crash_at_its_full_length_all_zeros() {
    for seed in 0..SWEEP {
        let image = after_completion(seed, torn(), |rt, io, file| {
            allocate(rt, io, file, GROWN as u64).expect("allocate");
            sync(rt, io, file).expect("force");
        });
        assert_eq!(GROWN, image.len(), "seed {seed}");
        assert_eq!([1; OLD], image[..OLD], "seed {seed}");
        assert!(image[OLD..].iter().all(|&b| b == 0), "seed {seed}");
    }
}

#[test]
fn an_unforced_allocate_leaves_the_old_length_or_more_and_only_zeros_past_it() {
    let (mut lost, mut partial, mut whole) = (false, false, false);
    for (faults, name) in [(Faults::default(), "default"), (torn(), "torn")] {
        for seed in 0..SWEEP {
            let image = after_completion(seed, faults.clone(), |rt, io, file| {
                allocate(rt, io, file, GROWN as u64).expect("allocate");
            });
            let at = format!("{name} seed {seed}");
            assert!(
                (OLD..=GROWN).contains(&image.len()),
                "{at}: {}",
                image.len()
            );
            assert_eq!([1; OLD], image[..OLD], "{at}");
            assert!(image[OLD..].iter().all(|&b| b == 0), "{at}");
            lost |= image.len() == OLD;
            partial |= OLD < image.len() && image.len() < GROWN;
            whole |= image.len() == GROWN;
        }
    }
    assert!(lost && partial && whole, "{lost} {partial} {whole}");
}

#[test]
fn an_unforced_allocate_keeps_its_full_length_when_completed_writes_survive() {
    for seed in 0..SWEEP {
        let image = after_completion(seed, kept(), |rt, io, file| {
            allocate(rt, io, file, GROWN as u64).expect("allocate");
        });
        assert_eq!(GROWN, image.len(), "seed {seed}");
    }
}

#[test]
fn a_forced_write_inside_an_allocated_range_keeps_its_bytes_under_a_later_torn_allocate() {
    let inside = 2 * UNIT;
    for seed in 0..SWEEP {
        let image = after_completion(seed, torn(), |rt, io, file| {
            allocate(rt, io, file, GROWN as u64).expect("allocate");
            write(rt, io, file, inside as u64, &sector(7)).expect("write");
            sync(rt, io, file).expect("force");
            allocate(rt, io, file, 2 * GROWN as u64).expect("grow unforced");
        });
        assert!(image.len() >= GROWN, "seed {seed}: {}", image.len());
        assert_eq!(sector(7), image[inside..inside + SECTOR], "seed {seed}");
        assert!(image[OLD..inside].iter().all(|&b| b == 0), "seed {seed}");
        assert!(
            image[inside + SECTOR..].iter().all(|&b| b == 0),
            "seed {seed}"
        );
    }
}

#[test]
fn a_crash_during_an_allocate_leaves_the_old_image() {
    for seed in 0..SWEEP {
        let image = during(seed, kept(), |file| Op::Allocate {
            file,
            len: GROWN as u64,
        });
        assert_eq!([1; OLD], *image, "seed {seed}");
    }
}

/// Returns, for each atomic unit of a `GROWN` write of 2 in flight over `OLD` bytes of 1, whether
/// it landed, after checking it landed whole or left what was there.
fn in_flight_units(seed: u64, faults: Faults) -> Vec<bool> {
    let image = during(seed, faults, |file| Op::Write {
        file,
        offset: 0,
        buf: AlignedBuf::from_slice(&[2; GROWN]),
    });
    assert!((OLD..=GROWN).contains(&image.len()), "seed {seed}");
    (0..GROWN.div_ceil(UNIT))
        .map(|u| {
            let start = u * UNIT;
            let part = &image[start.min(image.len())..(start + UNIT).min(image.len())];
            let landed = part.first() == Some(&2);
            let untouched = |i: usize| u8::from(start + i < OLD);
            assert!(
                part.iter().enumerate().all(|(i, &b)| if landed {
                    b == 2
                } else {
                    b == untouched(i)
                }),
                "seed {seed}: unit {u}"
            );
            landed
        })
        .collect()
}

#[test]
fn a_crash_during_a_write_lands_each_atomic_unit_whole_or_not_at_all() {
    let (mut mixed, mut last_landed) = (false, false);
    for seed in 0..SWEEP {
        let units = in_flight_units(seed, torn());
        mixed |= units.contains(&true) && units.contains(&false);
        last_landed |= units[3];
    }
    assert!(mixed && last_landed, "{mixed} {last_landed}");
}

#[test]
fn a_crash_during_a_write_may_lose_it_even_when_completed_writes_survive() {
    let (mut landed, mut lost) = (false, false);
    for seed in 0..SWEEP {
        let units = in_flight_units(seed, kept());
        landed |= units.iter().all(|&u| u);
        lost |= units.iter().all(|&u| !u);
    }
    assert!(landed && lost, "{landed} {lost}");
}
