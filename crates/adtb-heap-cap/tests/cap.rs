//! The cap as a test binary sees it: a runaway dies alone, and no test binary goes without the cap.

use adtb_heap_cap::{DEFAULT_CAP, current_cap, raise_cap, slots_for, test_child};
use std::fs;
use std::io::Write;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Output;

const MODE: &str = "ADTB_HEAP_CAP_TEST_MODE";
const CAP: &str = "ADTB_TEST_HEAP_CAP";
const PHASE: &str = "NEXTEST_TEST_PHASE";
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
        "print_cap" => {
            writeln!(std::io::stdout(), "cap {}", current_cap()).expect("stdout takes the cap");
        }
        "spawn_a_child" => {
            drop(test_child("true", 1));
        }
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
    let mut command = test_child(
        std::env::current_exe().expect("the test binary has a path"),
        1,
    );
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
    let mut command = test_child(
        std::env::current_exe().expect("the test binary has a path"),
        1,
    );
    command
        .args(["--exact", "child_entry", "--nocapture"])
        .env(MODE, "core_limit");
    // SAFETY: the hook makes one async-signal-safe call and allocates only on failure.
    unsafe { command.pre_exec(move || set_core_limit(limit)) };
    let child = command.output().expect("the child starts");
    assert_entry_ran(&child);
}

#[test]
fn every_test_binary_has_the_heap_cap() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    assert_eq!(
        adtb_heap_cap::uncapped_test_binaries(&manifest),
        Vec::<String>::new()
    );
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

/// Returns the module path, with a trailing `::`, of a source under `tests/BINARY/`, a module of
/// the test binary rooted at `tests/BINARY/main.rs`, and the empty string for any other source.
fn module_prefix(source: &Path) -> String {
    let parts: Vec<_> = source.iter().filter_map(|part| part.to_str()).collect();
    let Some(tests) = parts.iter().rposition(|part| *part == "tests") else {
        return String::new();
    };
    let Some(below_binary) = parts.get(tests + 2..) else {
        return String::new();
    };
    below_binary
        .iter()
        .map(|part| part.strip_suffix(".rs").unwrap_or(part))
        .filter(|module| !matches!(*module, "main" | "mod"))
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
        .filter(|(name, bytes)| !overrides.contains(&(name.clone(), slots_for(*bytes))))
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

/// Returns the `slots-N` group of each override of a nextest config whose `threads-required` is
/// not N.
fn groups_without_their_slots(config: &str) -> Vec<String> {
    config
        .split("[[")
        .filter_map(|block| {
            let setting = |name: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix(" = "))
            };
            let group = setting("test-group")?.trim_matches('"');
            let slots = group.strip_prefix("slots-")?;
            (setting("threads-required") != Some(slots)).then(|| group.to_owned())
        })
        .collect()
}

#[test]
fn a_slots_group_without_its_slots_is_reported() {
    let two = "[[o]]\nfilter = \"x\"\nthreads-required = 2\ntest-group = \"slots-2\"\n";
    assert_eq!(groups_without_their_slots(two), Vec::<String>::new());
    let one =
        "[[o]]\nthreads-required = 1\ntest-group = \"slots-2\"\n[[o]]\nthreads-required = 2\n";
    assert_eq!(groups_without_their_slots(one), ["slots-2"]);
    assert_eq!(
        groups_without_their_slots("[[o]]\ntest-group = \"slots-3\"\n"),
        ["slots-3"]
    );
}

#[test]
fn every_slots_group_holds_its_slots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dev_shell = fs::read_to_string(root.join("devenv.nix")).expect("devenv.nix reads");
    assert!(
        !dev_shell.contains("--config-file"),
        "check runs each workspace with its own nextest config"
    );
    for config in [".config/nextest.toml", "apps/campaign/.config/nextest.toml"] {
        let config = fs::read_to_string(root.join(config)).expect("the config reads");
        assert!(config.contains("test-group = \"slots-2\""), "{config}");
        assert_eq!(groups_without_their_slots(&config), Vec::<String>::new());
    }
}

#[test]
fn the_helpers_child_starts_with_the_cap_it_computed() {
    let child = test_child(
        std::env::current_exe().expect("the test binary has a path"),
        4,
    )
    .args(["--exact", "child_entry", "--nocapture"])
    .env(MODE, "print_cap")
    .output()
    .expect("the child starts");
    assert!(child.status.success(), "{child:?}");
    let stdout = String::from_utf8_lossy(&child.stdout);
    let printed = stdout
        .lines()
        .find_map(|line| line.strip_prefix("cap "))
        .unwrap_or_else(|| panic!("no cap line in {stdout}"));
    let expected = std::env::var(CAP).unwrap_or_else(|_| (DEFAULT_CAP / 4).to_string());
    assert_eq!(printed, expected);
}

#[test]
fn a_test_in_no_slots_group_cannot_build_a_child() {
    let child = test_child(
        std::env::current_exe().expect("the test binary has a path"),
        1,
    )
    .args(["--exact", "child_entry", "--nocapture"])
    .env(MODE, "spawn_a_child")
    .env(PHASE, "run")
    .env("NEXTEST_TEST_GROUP", "@global")
    .output()
    .expect("the child starts");
    assert!(!child.status.success(), "{child:?}");
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(stderr.contains("`slots-N` test group"), "{stderr}");
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
    let computed = raised_tests("fn heavy() {\n    raise_cap(BYTES);\n}\n");
    assert_eq!(unbacked(&computed, &enough), ["heavy"]);
}

#[test]
fn every_raised_test_holds_its_slots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    rust_sources(&root.join("crates"), &mut sources);
    rust_sources(&root.join("apps"), &mut sources);
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
