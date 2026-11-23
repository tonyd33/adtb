//! Search a workload's failure paths: fail each op it submits, take each image a crash at each of
//! its device events can leave, or act once at each op.

use std::collections::{BTreeMap, BTreeSet};

use crate::profile::{assert_reached, count, every_case, spread};
use adtb_sim::{Action, Config, Outcome, Plan, Sim, SimRuntime, Struck};
use adtb_sys::runtime::{IoError, OpKind};

use crate::{crash, process};

pub type Files = BTreeMap<String, Vec<u8>>;

/// How many times the images a crash search draws around a rename, where the order of names counts.
const RENAME_IMAGES: usize = 4;

fn is_bulk(kind: OpKind) -> bool {
    matches!(kind, OpKind::Read | OpKind::Write | OpKind::Size)
}

fn numbered(kinds: &[OpKind], pick: impl Fn(OpKind) -> bool) -> Vec<u64> {
    (0..kinds.len() as u64)
        .filter(|&i| pick(kinds[i as usize]))
        .collect()
}

/// Returns each rename and the ops beside it, up to `most` of the ops that change no bytes and two
/// of those that do or only read.
fn around_renames(kinds: &[OpKind], most: u64) -> BTreeSet<u64> {
    let last = kinds.len() as u64;
    let renames = numbered(kinds, |kind| kind == OpKind::Rename);
    let mut out: BTreeSet<u64> = renames
        .into_iter()
        .flat_map(|at| [at.saturating_sub(1), at, at + 1])
        .filter(|&at| at < last)
        .collect();
    out.extend(spread(&numbered(kinds, |k| !is_bulk(k)), count(most)));
    out.extend(spread(&numbered(kinds, is_bulk), count(2)));
    out
}

/// How many ops of each kind a failure search fails, as [`count`] sizes it.
const PER_KIND: u64 = 1;

/// Returns each kind of `kinds` once.
fn distinct(kinds: &[OpKind]) -> BTreeSet<OpKind> {
    kinds.iter().copied().collect()
}

/// Returns the number of the first rename of `kinds`, or their count when none is.
fn first_rename(kinds: &[OpKind]) -> usize {
    kinds
        .iter()
        .position(|&k| k == OpKind::Rename)
        .unwrap_or(kinds.len())
}

/// Returns whether an op of `kind` makes, removes, renames or forces a name.
fn names(kind: OpKind) -> bool {
    matches!(
        kind,
        OpKind::CreateNew
            | OpKind::CreateDir
            | OpKind::Remove
            | OpKind::RemoveDir
            | OpKind::Rename
            | OpKind::SyncDir
    )
}

/// Returns the op numbers a search at every name fails: every op that [`names`], and every op from
/// the first rename on.
fn every_named(kinds: &[OpKind]) -> BTreeSet<u64> {
    let mut out: BTreeSet<u64> = numbered(kinds, names).into_iter().collect();
    out.extend(first_rename(kinds) as u64..kinds.len() as u64);
    out
}

/// Which failure runs a search makes and how it judges them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The workload finishes after every failure; the profile `deep` fails at every name.
    Finishing,
    /// The process may crash on a failure; the profile `deep` fails at every name.
    Fatal,
    /// The process may crash on a failure, and every profile fails at every name.
    FatalAtEveryName,
}

impl Mode {
    fn at_every_name(self) -> bool {
        self == Mode::FatalAtEveryName || every_case()
    }
}

/// Returns the op numbers to fail: [`around_renames`]'s, and when `mode` fails at every name
/// [`every_named`]'s and [`PER_KIND`] of each kind, evenly spread.
fn chosen_ops(kinds: &[OpKind], most: u64, mode: Mode) -> Vec<u64> {
    let mut out = around_renames(kinds, most);
    if !mode.at_every_name() {
        return out.into_iter().collect();
    }
    for kind in distinct(kinds) {
        out.extend(spread(&numbered(kinds, |k| k == kind), count(PER_KIND)));
    }
    out.extend(every_named(kinds));
    out.into_iter().collect()
}

/// Returns the op numbers to start a second run at: [`around_renames`]'s, and under the profile
/// `deep` each op that changes no bytes.
fn chosen_starts(kinds: &[OpKind], most: u64) -> Vec<u64> {
    let mut out = around_renames(kinds, most);
    if every_case() {
        out.extend(numbered(kinds, |k| !is_bulk(k)));
    }
    out.into_iter().collect()
}

/// One workload run with a failure at one op.
#[derive(Debug)]
pub struct Failed<T> {
    /// The number of the failed op.
    pub at: u64,
    pub struck: Struck,
    /// What the workload returned.
    pub out: T,
    /// The device after the workload, as a reader sees it.
    pub files: Files,
    pub dirs: BTreeSet<String>,
}

/// Returns the kind of each op `workload` numbers on a device holding `files`, run with no fault.
fn eligible_ops<T>(files: &Files, seed: u64, workload: &dyn Fn(&SimRuntime) -> T) -> Vec<OpKind> {
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        workload(&rt);
    });
    sim.numbered().kinds().to_vec()
}

/// Returns the error the failure search fails an op of `kind` with.
fn failure(kind: OpKind) -> IoError {
    match kind {
        OpKind::Write | OpKind::Allocate => IoError::Full,
        _ => IoError::Failed,
    }
}

/// Run `workload` on a device holding `files` failing each chosen op, and give `check` each result.
///
/// Preconditions:
/// - `workload` calls [`SimRuntime::arm`] where the search starts and [`SimRuntime::disarm`]
///   where it ends, and is deterministic for a seed up to the failure. A plan of its own panics.
pub fn each_failure<T>(
    files: &Files,
    seed: u64,
    most: u64,
    workload: &dyn Fn(&SimRuntime) -> T,
    check: &mut dyn FnMut(Failed<T>),
) {
    failure_runs(
        files,
        seed,
        most,
        &[],
        workload,
        Mode::Finishing,
        &mut |failed| {
            check(Failed {
                at: failed.at,
                struck: failed.struck,
                out: failed.out.expect("the workload finishes"),
                files: failed.files,
                dirs: failed.dirs,
            });
        },
    );
}

/// Search failures as [`each_failure`] does, accepting typed fatal I/O after the simulator's
/// crash lifecycle. Return `None` instead of a workload result when the process crashes.
/// Fail the positions [`chosen_ops`] samples, with the first, the last and `also`.
pub fn each_fatal_failure<T>(
    files: &Files,
    seed: u64,
    most: u64,
    also: &[u64],
    workload: &dyn Fn(&SimRuntime) -> T,
    check: &mut dyn FnMut(Failed<Option<T>>),
) {
    failure_runs(files, seed, most, also, workload, Mode::Fatal, check);
}

/// Search failures as [`each_fatal_failure`] does, failing in every profile each op that makes,
/// removes, renames or forces a name and each op from the first rename on.
pub fn each_fatal_failure_at_every_name<T>(
    files: &Files,
    seed: u64,
    most: u64,
    also: &[u64],
    workload: &dyn Fn(&SimRuntime) -> T,
    check: &mut dyn FnMut(Failed<Option<T>>),
) {
    failure_runs(
        files,
        seed,
        most,
        also,
        workload,
        Mode::FatalAtEveryName,
        check,
    );
}

fn failure_runs<T>(
    files: &Files,
    seed: u64,
    most: u64,
    also: &[u64],
    workload: &dyn Fn(&SimRuntime) -> T,
    mode: Mode,
    check: &mut dyn FnMut(Failed<Option<T>>),
) {
    let fatal = mode != Mode::Finishing;
    let kinds = eligible_ops(files, seed, workload);
    assert!(!kinds.is_empty(), "the workload arms the search");
    let total = kinds.len();
    let mut positions: BTreeSet<_> = chosen_ops(&kinds, most, mode).into_iter().collect();
    if fatal {
        positions.extend([0, (total - 1) as u64]);
        positions.extend(also);
    }
    let mut struck_at: BTreeMap<u64, OpKind> = BTreeMap::new();
    for at in positions {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        let error = failure(kinds[at as usize]);
        sim.own_plan(
            "search::each_failure",
            Plan::new().at(at, Action::Fail(error)),
        );
        let mut out = None;
        let body = |rt: SimRuntime| {
            out = Some(workload(&rt));
        };
        let outcome = if fatal {
            crash::run_process(&mut sim, Box::new(body))
        } else {
            sim.run(body)
        };
        match outcome {
            Outcome::Finished => assert!(out.is_some(), "the workload returns"),
            Outcome::Crashed => {
                assert!(fatal, "the workload finishes");
                out = None;
            }
        }
        let failed = sim
            .numbered()
            .struck()
            .cloned()
            .unwrap_or_else(|| panic!("op {at} of {total} came up in the workload"));
        struck_at.insert(at, failed.kind());
        check(Failed {
            at,
            struck: failed,
            out,
            files: sim.files(),
            dirs: sim.dirs(),
        });
    }
    if !mode.at_every_name() {
        return;
    }
    for at in every_named(&kinds) {
        assert_eq!(
            Some(&kinds[at as usize]),
            struck_at.get(&at),
            "op {at} of {total}, which a search at every name fails, failed as numbered"
        );
    }
    for kind in distinct(&kinds) {
        let present = numbered(&kinds, |k| k == kind);
        let reached = present
            .iter()
            .filter(|at| struck_at.get(at) == Some(&kind))
            .count();
        assert_reached(
            &format!("a failed {kind:?}"),
            reached,
            present.len().min(count(PER_KIND) as usize),
        );
    }
}

/// Run `workload` once and give `check` each image a crash at each chosen device event after it arms
/// the search can leave, up to `images` for each event, with the event's number. Fail unless the
/// images include one `published` accepts and one it does not.
pub fn each_crash<T>(
    files: &Files,
    seed: u64,
    most: u64,
    images: usize,
    workload: Box<dyn FnOnce(&SimRuntime) -> T + '_>,
    published: &dyn Fn(&Files) -> bool,
    check: &mut dyn FnMut(usize, Files),
) {
    let (mut before, mut after) = (0, 0);
    let run = |rt: &SimRuntime| {
        workload(rt);
        Vec::new()
    };
    each_crash_image(
        files,
        seed,
        most,
        images,
        Box::new(run),
        &mut |at, image| {
            if published(&image) {
                after += 1;
            } else {
                before += 1;
            }
            check(at, image);
        },
    );
    assert_reached("an image before the result is published", before, 1);
    assert_reached("an image with the result published", after, 1);
}

/// A workload that returns the armed device events a crash search must include.
pub type ArmedEvents<'a> = Box<dyn FnOnce(&SimRuntime) -> Vec<usize> + 'a>;

/// Run `workload` once and give `check` each image a crash at each chosen device event after it arms
/// the search can leave, up to `images` for each event, with the event's number. The chosen events
/// include each armed one `workload` returns.
pub fn each_crash_image(
    files: &Files,
    seed: u64,
    most: u64,
    images: usize,
    workload: ArmedEvents<'_>,
    check: &mut dyn FnMut(usize, Files),
) {
    each_crash_image_under(
        Config::default(),
        files,
        seed,
        most,
        images,
        workload,
        check,
    );
}

/// Search as [`each_crash_image`] does, running `workload` under `config`.
pub fn each_crash_image_under(
    config: Config,
    files: &Files,
    seed: u64,
    most: u64,
    images: usize,
    workload: ArmedEvents<'_>,
    check: &mut dyn FnMut(usize, Files),
) {
    let mut sim = Sim::with_files(seed, config, files.clone());
    sim.record();
    let also = process::run(&mut sim, |rt| workload(&rt));
    let first = sim.numbered().armed_at();
    let recording = sim.recording();
    let every: Vec<u64> = (first as u64..=recording.len() as u64).collect();
    let renames = recording.renames().into_iter().map(|at| at as u64);
    let around: BTreeSet<u64> = renames
        .flat_map(|at| [at - 1, at, at + 1])
        .filter(|at| every.contains(at))
        .collect();
    let points: BTreeSet<u64> = if every_case() {
        every.iter().copied().collect()
    } else {
        around
            .iter()
            .copied()
            .chain(spread(&every, count(most)))
            .chain(also.iter().map(|&at| at as u64))
            .collect()
    };
    for at in &also {
        assert!(every.contains(&(*at as u64)), "event {at} is armed");
    }
    for at in points {
        let drawn = if around.contains(&at) {
            images * RENAME_IMAGES
        } else {
            images
        };
        let at = at as usize;
        for image in recording.images(at, drawn, at as u64) {
            check(at, image);
        }
    }
}

/// Run `workload` once with no op to act at, to count, then once per chosen op with its number. The
/// workload plans its own action at that number with [`SimRuntime::plan`].
pub fn each_start(
    files: &Files,
    seed: u64,
    most: u64,
    workload: &dyn Fn(&SimRuntime, Option<u64>),
    after: &mut dyn FnMut(u64, Files),
) {
    let kinds = eligible_ops(files, seed, &|rt: &SimRuntime| workload(rt, None));
    assert!(!kinds.is_empty(), "the workload arms the search");
    let total = kinds.len();
    for at in chosen_starts(&kinds, most) {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        process::run(&mut sim, |rt| workload(&rt, Some(at)));
        assert!(
            sim.numbered().struck().is_some(),
            "op {at} of {total} came up in the workload"
        );
        after(at, sim.files());
    }
}
