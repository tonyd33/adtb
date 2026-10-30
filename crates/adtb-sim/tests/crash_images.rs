//! What a crash leaves on the device, pinned byte for byte on fixed seeds. A difference means the
//! simulator's bytes or its random draws changed.

use adtb_simkit::common;

use std::collections::BTreeMap;
use std::time::Duration;

use adtb::runtime::{Done, FileId, IoError, Op, Runtime};
use adtb_sim::{Config, Faults, Outcome, Sim, SimIo, SimRuntime};
use common::{SECTOR, complete, open, read, run, sector, sync, sync_dir, write};

const SEEDS: u64 = 4;
const MIB: u64 = 1 << 20;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fold(digest: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(digest, |d, &b| (d ^ u64::from(b)).wrapping_mul(FNV_PRIME))
}

fn fold_files(mut digest: u64, files: &BTreeMap<String, Vec<u8>>) -> u64 {
    for (path, bytes) in files {
        digest = fold(digest, path.as_bytes());
        digest = fold(digest, &(bytes.len() as u64).to_le_bytes());
        digest = fold(digest, bytes);
    }
    digest
}

fn allocate(rt: &SimRuntime, io: &mut SimIo, file: FileId, len: u64) -> Result<(), IoError> {
    complete(rt, io, Op::Allocate { file, len })
        .result
        .map(|done| assert_eq!(Done::Allocated, done))
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
    let digest = fold(FNV_OFFSET, &failed.to_le_bytes());
    let digest = fold(digest, sim.trace().as_bytes());
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
            0x733e_79bf_5c2e_224a,
            0xc692_3580_438e_c156,
            0x0961_bb6f_fc0e_8d8b,
            0xf724_485b_ea1a_164e,
        ],
    );
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
            fold_files(fold(FNV_OFFSET, sim.trace().as_bytes()), &sim.files())
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
