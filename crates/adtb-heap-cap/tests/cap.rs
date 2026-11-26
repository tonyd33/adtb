//! The cap as a test binary sees it: a runaway dies alone, and no test binary goes without the cap.

use adtb_heap_cap::{DEFAULT_CAP, SLACK, live, raise_cap, slots_for, take_peak};
use std::fs;
use std::io::Write as _;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Barrier;
use std::thread;

#[path = "../../../xtask/src/listing.rs"]
mod listing;

const MODE: &str = "ADTB_HEAP_CAP_TEST_MODE";
const CAP: &str = "ADTB_TEST_HEAP_CAP";
const SMALL_BOX_CAP: usize = 268_435_456;

fn core_limit() -> libc::rlimit {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: the pointer names an `rlimit` the call fills.
    let status = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &raw mut limit) };
    assert_eq!(status, 0);
    limit
}

fn hold_small_boxes<T: Default>() -> ! {
    let mut held = Vec::new();
    loop {
        held.push(Box::new(T::default()));
    }
}

fn run_mode(mode: &str) {
    if let Some(threads) = mode.strip_prefix("slack ") {
        let (running, exited) = threads.split_once(' ').expect("two thread counts");
        let count = |n: &str| n.parse().expect("a thread count");
        return hold_past_the_cap(count(running), count(exited));
    }
    match mode {
        "megabytes" => {
            let mut held = Vec::new();
            loop {
                held.push(vec![1u8; 1 << 20]);
            }
        }
        "box1" => hold_small_boxes::<u8>(),
        "box8" => hold_small_boxes::<u64>(),
        "past_default" => {
            std::hint::black_box(vec![0u8; DEFAULT_CAP + (1 << 20)]);
        }
        "raised_past_default" => {
            raise_cap(2 * DEFAULT_CAP);
            std::hint::black_box(vec![0u8; DEFAULT_CAP + (1 << 20)]);
        }
        "core_limit" => assert_eq!(core_limit().rlim_cur, 0),
        other => panic!("no mode {other}"),
    }
}

#[test]
fn child_entry() {
    if let Ok(mode) = std::env::var(MODE) {
        run_mode(&mode);
    }
}

fn child(mode: &str, cap: Option<usize>) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("the test binary has a path"));
    command
        .args(["--exact", "child_entry", "--nocapture"])
        .env(MODE, mode)
        .env("RUST_TEST_THREADS", "2");
    match cap {
        Some(cap) => command.env(CAP, cap.to_string()),
        None => command.env_remove(CAP),
    };
    command.output().expect("the child starts")
}

/// Assert the child ran `child_entry` and passed: `--exact` with no such test also exits 0.
fn assert_entry_ran(child: &Output) {
    assert!(child.status.success(), "{child:?}");
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(stdout.contains(" 1 passed;"), "{stdout}");
}

fn cap_line(child: &Output) -> String {
    assert_eq!(child.status.signal(), Some(libc::SIGABRT), "{child:?}");
    let stderr = String::from_utf8_lossy(&child.stderr);
    stderr
        .lines()
        .find(|line| line.starts_with("adtb-heap-cap: "))
        .unwrap_or_else(|| panic!("no cap line in {stderr}"))
        .to_owned()
}

#[test]
fn a_runaway_aborts_at_the_cap_and_the_parent_lives() {
    let line = cap_line(&child("megabytes", Some(67_108_864)));
    assert!(line.ends_with("past the cap of 67108864"), "{line}");
    assert!(
        line.starts_with("adtb-heap-cap: thread child_entry: "),
        "{line}"
    );
}

fn resident_of_children() -> usize {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: the pointer names a `rusage` the call fills.
    let status = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) };
    assert_eq!(status, 0);
    // SAFETY: a zero status means the call filled it, and all zeroes is a valid `rusage`.
    let kib = unsafe { usage.assume_init() }.ru_maxrss;
    usize::try_from(kib).expect("a resident size is positive") * 1024
}

fn assert_resident_stays_near_the_cap(mode: &str) {
    cap_line(&child(mode, Some(SMALL_BOX_CAP)));
    let resident = resident_of_children();
    assert!(
        resident < SMALL_BOX_CAP * 3 / 2,
        "{mode}: {resident} bytes resident at a cap of {SMALL_BOX_CAP}"
    );
}

#[test]
fn one_byte_boxes_stay_near_the_cap_in_resident_memory() {
    assert_resident_stays_near_the_cap("box1");
}

#[test]
fn eight_byte_boxes_stay_near_the_cap_in_resident_memory() {
    assert_resident_stays_near_the_cap("box8");
}

#[test]
fn the_default_cap_aborts_a_test_that_did_not_raise_it() {
    let line = cap_line(&child("past_default", None));
    assert!(
        line.ends_with(&format!("past the cap of {DEFAULT_CAP}")),
        "{line}"
    );
}

#[test]
fn a_raised_cap_lets_the_test_pass_the_default() {
    let child = child("raised_past_default", None);
    assert_entry_ran(&child);
}

#[test]
fn the_environment_overrides_a_raised_cap() {
    let line = cap_line(&child("raised_past_default", Some(67_108_864)));
    assert!(line.ends_with("past the cap of 67108864"), "{line}");
}

/// A block whose malloc chunk is [`BLOCK_COST`] bytes: 4080 and the 8 byte header, rounded to 16.
type Block = Box<[u8; 4080]>;

const BLOCK_COST: usize = 4096;

/// The cap the slack search runs under, in bytes.
const SLACK_SEARCH_CAP: usize = 8 << 20;

/// The threads of each slack case: running and holding, then exited after holding.
const SLACK_CASES: [(usize, usize); 4] = [(0, 0), (8, 0), (0, 16), (8, 16)];

/// The blocks each holder holds: their chunks cost just under [`SLACK`].
const HELD_BLOCKS: usize = (SLACK - 1) / BLOCK_COST;

/// The blocks the allocating thread makes at most: twice the cap.
const CEILING: usize = 2 * SLACK_SEARCH_CAP / BLOCK_COST;

/// Returns what malloc takes for a block of `size` bytes at an alignment of 16 or less.
fn chunk(size: usize) -> usize {
    ((size + 8 + 15) & !15).max(32)
}

fn hold() -> Vec<Block> {
    let mut held = Vec::with_capacity(HELD_BLOCKS);
    held.extend((0..HELD_BLOCKS).map(|_| Block::new([1; 4080])));
    held
}

/// Returns the bytes a holder holds, its vector's chunk with its blocks.
fn held_by_each_thread() -> usize {
    chunk(HELD_BLOCKS * size_of::<Block>()) + HELD_BLOCKS * BLOCK_COST
}

/// Report a count on stderr as `NAME N`.
fn report(name: &str, n: usize) {
    writeln!(std::io::stderr(), "{name} {n}").expect("stderr takes a line");
}

/// Hold just under [`SLACK`] on `running` threads that stay and on `exited` threads that have
/// exited, then allocate blocks on this thread, reporting each, until the cap aborts the process.
fn hold_past_the_cap(running: usize, exited: usize) {
    report("before", live());
    let holding = Barrier::new(running + 1);
    let done = Barrier::new(running + 1);
    thread::scope(|scope| {
        let kept: Vec<Vec<Block>> = (0..exited)
            .map(|_| scope.spawn(hold).join().expect("a holder joins"))
            .collect();
        for _ in 0..running {
            scope.spawn(|| {
                let held = hold();
                holding.wait();
                done.wait();
                drop(held);
            });
        }
        holding.wait();
        let mut blocks = Vec::with_capacity(CEILING);
        while blocks.len() < CEILING {
            blocks.push(Block::new([1; 4080]));
            report("blocks", blocks.len());
        }
        done.wait();
        drop((kept, blocks));
    });
}

/// Returns the last count `name` reports on the child's stderr.
fn reported(child: &Output, name: &str) -> usize {
    let stderr = String::from_utf8_lossy(&child.stderr);
    stderr
        .lines()
        .filter_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
        .next_back()
        .unwrap_or_else(|| panic!("no {name} count in {stderr}"))
        .parse()
        .expect("a count")
}

/// Returns the blocks the allocating thread made before the cap aborted case `(running, exited)`,
/// after checking the bytes the child held when it aborted against the stated slack.
fn blocks_at_the_abort(running: usize, exited: usize) -> usize {
    let child = child(&format!("slack {running} {exited}"), Some(SLACK_SEARCH_CAP));
    let line = cap_line(&child);
    let case = format!("{running} running and {exited} exited holders: {line}");
    assert!(
        line.ends_with(&format!("past the cap of {SLACK_SEARCH_CAP}")),
        "{case}"
    );
    let blocks = reported(&child, "blocks");
    let kept = if exited == 0 {
        0
    } else {
        chunk(exited * size_of::<Vec<Block>>())
    };
    let held = reported(&child, "before")
        + kept
        + (running + exited) * held_by_each_thread()
        + chunk(CEILING * size_of::<Block>())
        + (blocks + 1) * BLOCK_COST;
    // The holders, this thread and the harness's main thread, whose held bytes `before` misses.
    let threads = running + 2;
    let most = SLACK_SEARCH_CAP + threads * SLACK + BLOCK_COST;
    assert!(held <= most, "{case}: {held} bytes held past {most}");
    let least = SLACK_SEARCH_CAP - (2 * threads + 1) * SLACK;
    assert!(
        held > least,
        "{case}: aborted at {held} bytes, short of {least}"
    );
    blocks
}

#[test]
fn a_process_aborts_within_the_slack_of_each_running_thread_of_its_cap() {
    let alone = blocks_at_the_abort(0, 0);
    for (running, exited) in SLACK_CASES {
        let blocks = blocks_at_the_abort(running, exited);
        let counted = alone - exited * HELD_BLOCKS;
        assert!(
            blocks.abs_diff(counted) <= HELD_BLOCKS,
            "{running} running and {exited} exited holders: {blocks} blocks at the abort, \
             not near the {counted} the exited holders' count leaves"
        );
    }
}

#[test]
fn the_peak_of_one_thread_is_exact_across_the_slack() {
    for n in [1, 10, 65, 100, 200] {
        let mut blocks = Vec::with_capacity(n);
        take_peak();
        let before = live();
        blocks.extend((0..n).map(|_| Block::new([1; 4080])));
        blocks.clear();
        assert_eq!(take_peak() - before, n * BLOCK_COST, "{n} blocks");
        assert_eq!(live(), before, "{n} blocks freed");
    }
}

fn set_core_limit(limit: libc::rlimit) -> std::io::Result<()> {
    // SAFETY: the pointer names an initialised `rlimit`.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raw const limit) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Under a hard limit of zero the child starts at zero and this proves nothing.
#[test]
fn a_test_binary_started_with_core_dumps_on_zeroes_its_core_limit() {
    let mut limit = core_limit();
    limit.rlim_cur = limit.rlim_max.min(4096);
    let mut command = Command::new(std::env::current_exe().expect("the test binary has a path"));
    command
        .args(["--exact", "child_entry", "--nocapture"])
        .env(MODE, "core_limit");
    // SAFETY: the hook makes one async-signal-safe call and allocates only on failure.
    unsafe { command.pre_exec(move || set_core_limit(limit)) };
    let child = command.output().expect("the child starts");
    assert_entry_ran(&child);
}

/// Returns the paths of the test binaries of the workspace of `manifest` that do not abort at a
/// one byte cap, building them first unless `ADTB_TEST_BINARIES` names their listing, a path to a
/// line.
///
/// # Panics
///
/// When `cargo nextest list` fails, the named listing does not read, no binary is listed, or
/// `cargo xtask` started the test and named no listing.
fn uncapped_test_binaries(manifest: &Path) -> Vec<String> {
    let binaries: Vec<String> = match std::env::var_os(listing::TEST_BINARIES) {
        Some(path) => fs::read_to_string(&path)
            .unwrap_or_else(|e| {
                panic!(
                    "{} names {}: {e}",
                    listing::TEST_BINARIES,
                    Path::new(&path).display()
                )
            })
            .lines()
            .map(str::to_owned)
            .collect(),
        None => {
            assert_eq!(
                std::env::var_os(listing::UNDER_XTASK),
                None,
                "cargo xtask started this test and set no {}",
                listing::TEST_BINARIES
            );
            let listed = list_command(manifest).output().expect("cargo nextest runs");
            assert!(
                listed.status.success(),
                "cargo nextest list fails: {}",
                String::from_utf8_lossy(&listed.stderr)
            );
            String::from_utf8_lossy(&listed.stdout)
                .split("\"binary-path\":\"")
                .skip(1)
                .filter_map(|rest| rest.split('"').next())
                .map(str::to_owned)
                .collect()
        }
    };
    assert!(!binaries.is_empty(), "no test binary is listed");
    binaries
        .into_iter()
        .filter(|binary| !aborts_at_one_byte(binary))
        .collect()
}

fn list_command(manifest: &Path) -> Command {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut list = Command::new(cargo);
    list.args([
        "nextest",
        "list",
        "--workspace",
        "--list-type",
        "binaries-only",
        "--message-format",
        "json",
        "--cargo-quiet",
        "--manifest-path",
    ])
    .arg(manifest)
    .env_remove("NEXTEST_PROFILE");
    list
}

fn aborts_at_one_byte(binary: &str) -> bool {
    Command::new(binary)
        .arg("--list")
        .env(CAP, "1")
        .output()
        .expect("the test binary starts")
        .status
        .signal()
        == Some(libc::SIGABRT)
}

#[test]
fn the_nested_list_drops_the_callers_profile() {
    let list = list_command(Path::new("Cargo.toml"));
    let profile = list.get_envs().find(|(name, _)| *name == "NEXTEST_PROFILE");
    assert_eq!(
        profile,
        Some((std::ffi::OsStr::new("NEXTEST_PROFILE"), None))
    );
}

#[test]
fn every_test_binary_has_the_heap_cap() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    assert_eq!(uncapped_test_binaries(&manifest), Vec::<String>::new());
}

/// Returns whether a line of the `[dev-dependencies]` table, comments aside, enables the feature.
fn enables_the_cap_for_tests(manifest: &str) -> bool {
    let mut in_dev_dependencies = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_dev_dependencies = line == "[dev-dependencies]";
        } else if in_dev_dependencies && line.contains("\"test-heap-cap\"") {
            return true;
        }
    }
    false
}

#[test]
fn a_comment_or_a_feature_definition_does_not_enable_the_cap() {
    let enabled =
        "[dev-dependencies]\nadtb-lang = { workspace = true, features = [\"test-heap-cap\"] }\n";
    assert!(enables_the_cap_for_tests(enabled));
    assert!(!enables_the_cap_for_tests(
        "[dev-dependencies]\n# adtb-lang features = [\"test-heap-cap\"]\n"
    ));
    assert!(!enables_the_cap_for_tests(
        "[dependencies]\nadtb-lang = { workspace = true, features = [\"test-heap-cap\"] }\n"
    ));
    assert!(!enables_the_cap_for_tests(
        "[features]\ntest-heap-cap = [\"dep:adtb-heap-cap\"]\n[dev-dependencies]\nother = \"1\"\n"
    ));
}

#[test]
fn every_package_enables_the_cap_for_its_own_tests() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut lacking = Vec::new();
    for package in fs::read_dir(crates).expect("the crates directory reads") {
        let path = package.expect("a directory entry").path();
        let manifest = fs::read_to_string(path.join("Cargo.toml")).expect("a manifest");
        if !path.ends_with("adtb-heap-cap") && !enables_the_cap_for_tests(&manifest) {
            lacking.push(path);
        }
    }
    assert_eq!(lacking, Vec::<PathBuf>::new());
}

/// Returns the bytes a `raise_cap` argument spells: `N` or `N << S`.
fn literal_bytes(argument: &str) -> Option<usize> {
    match argument.split_once("<<") {
        Some((n, shift)) => {
            let n: usize = n.trim().parse().ok()?;
            n.checked_shl(shift.trim().parse().ok()?)
        }
        None => argument.trim().parse().ok(),
    }
}

/// Returns each function of `source` that calls `raise_cap`, with the bytes it asks for, or
/// `usize::MAX` for an argument that is no literal.
fn raised_tests(source: &str) -> Vec<(String, usize)> {
    let mut raised = Vec::new();
    let mut function = "";
    for line in source.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("fn ")
            .or_else(|| line.strip_prefix("pub fn "))
        {
            function = rest.split(['(', '<']).next().unwrap_or_default();
        }
        if let Some((_, rest)) = line.split_once("raise_cap(") {
            let argument = rest.split(')').next().unwrap_or_default();
            let bytes = literal_bytes(argument).unwrap_or(usize::MAX);
            raised.push((function.to_owned(), bytes));
        }
    }
    raised
}

/// Returns the module path, with a trailing `::`, of a source under `src/`, a module of the
/// crate rooted at `src/lib.rs`, or under `tests/BINARY/`, a module of the test binary rooted at
/// `tests/BINARY/main.rs`, and the empty string for any other source.
fn module_prefix(source: &Path) -> String {
    let parts: Vec<_> = source.iter().filter_map(|part| part.to_str()).collect();
    let below = if let Some(src) = parts.iter().position(|part| *part == "src") {
        &parts[src + 1..]
    } else {
        let Some(tests) = parts.iter().rposition(|part| *part == "tests") else {
            return String::new();
        };
        let Some(below_binary) = parts.get(tests + 2..) else {
            return String::new();
        };
        below_binary
    };
    below
        .iter()
        .map(|part| part.strip_suffix(".rs").unwrap_or(part))
        .filter(|module| !matches!(*module, "main" | "mod" | "lib"))
        .map(|module| format!("{module}::"))
        .collect()
}

#[test]
fn a_module_of_a_test_binary_prefixes_its_tests() {
    assert_eq!(
        module_prefix(Path::new("crates/s/tests/storage/lost_bitmap.rs")),
        "lost_bitmap::"
    );
    assert_eq!(
        module_prefix(Path::new("crates/s/tests/execution/constraints/names.rs")),
        "constraints::names::"
    );
    assert_eq!(
        module_prefix(Path::new("crates/s/tests/storage/main.rs")),
        ""
    );
    assert_eq!(module_prefix(Path::new("crates/s/tests/cap.rs")), "");
    assert_eq!(module_prefix(Path::new("crates/s/src/lib.rs")), "");
    assert_eq!(
        module_prefix(Path::new("crates/s/src/storage/lost_bitmap_tests.rs")),
        "storage::lost_bitmap_tests::"
    );
    assert_eq!(
        module_prefix(Path::new("crates/s/src/plan/tests/mod.rs")),
        "plan::tests::"
    );
}

/// Returns each `test(=NAME)` override of a nextest config with its `threads-required`.
fn slot_overrides(config: &str) -> Vec<(String, usize)> {
    let mut overrides = Vec::new();
    let mut name = None;
    for line in config.lines() {
        if line.starts_with('[') {
            name = None;
        } else if let Some(filter) = line.strip_prefix("filter = \"test(=") {
            name = filter.strip_suffix(")\"");
        } else if let (Some(name), Some(slots)) = (name, line.strip_prefix("threads-required = ")) {
            overrides.push((name.to_owned(), slots.parse().expect("a slot count")));
        }
    }
    overrides
}

/// Returns the raised tests whose override does not hold the slots their cap covers.
fn unbacked(raised: &[(String, usize)], overrides: &[(String, usize)]) -> Vec<String> {
    raised
        .iter()
        .filter(|(name, bytes)| {
            !overrides
                .iter()
                .any(|(held, slots)| held == name && *slots >= slots_for(*bytes))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("a source directory reads") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            if !path.ends_with("target") && !path.ends_with("adtb-heap-cap") {
                rust_sources(&path, found);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn a_raise_without_its_slots_is_reported() {
    let raised = raised_tests("fn heavy() {\n    adtb_heap_cap::raise_cap(4 << 30);\n}\n");
    assert_eq!(raised, [("heavy".to_owned(), 4 << 30)]);
    assert_eq!(unbacked(&raised, &[]), ["heavy"]);
    let too_few = slot_overrides("[[o]]\nfilter = \"test(=heavy)\"\nthreads-required = 2\n");
    assert_eq!(too_few, [("heavy".to_owned(), 2)]);
    assert_eq!(unbacked(&raised, &too_few), ["heavy"]);
    let enough = slot_overrides("[[o]]\nfilter = \"test(=heavy)\"\nthreads-required = 3\n");
    assert_eq!(unbacked(&raised, &enough), Vec::<String>::new());
    let more = slot_overrides("[[o]]\nfilter = \"test(=heavy)\"\nthreads-required = 8\n");
    assert_eq!(unbacked(&raised, &more), Vec::<String>::new());
    let computed = raised_tests("fn heavy() {\n    raise_cap(BYTES);\n}\n");
    assert_eq!(unbacked(&computed, &enough), ["heavy"]);
}

#[test]
fn every_raised_test_holds_its_slots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    rust_sources(&root.join("crates"), &mut sources);
    let raised: Vec<_> = sources
        .iter()
        .flat_map(|path| {
            let prefix = module_prefix(path);
            raised_tests(&fs::read_to_string(path).expect("a source reads"))
                .into_iter()
                .map(move |(name, bytes)| (format!("{prefix}{name}"), bytes))
        })
        .collect();
    assert!(!raised.is_empty(), "the scan finds the raised tests");
    let config = fs::read_to_string(root.join(".config/nextest.toml")).expect("the config reads");
    assert_eq!(
        unbacked(&raised, &slot_overrides(&config)),
        Vec::<String>::new()
    );
}
