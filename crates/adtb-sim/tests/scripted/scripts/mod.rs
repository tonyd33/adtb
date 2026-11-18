//! Engine scripts: the scripts under `scripts/AREA/` run on the simulator, one test per area. The
//! directives are in the doc comment of `adtb_conformance::scripted`.

use adtb_testkit::scratch::Scratch;
use std::fs;
use std::path::{Path, PathBuf};

use adtb_conformance::Found;
use adtb_conformance::runner::{self, Corpus, Format, Options, Sides};
use adtb_conformance::scripted::run_file;

/// How many scripts run at once: the slots each area test holds.
const WORKERS: usize = 3;

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn root() -> PathBuf {
    crate_dir().join("scripts")
}

fn corpus() -> Corpus {
    Corpus::new(Format::Scripts, root())
}

/// Returns each difference of every script under `dir`, after each entry that is no script.
fn differences(dir: &Path) -> Vec<String> {
    let selection = runner::select(&corpus(), dir);
    let mut out: Vec<String> = selection.problems().iter().map(Found::shown).collect();
    for case in selection.cases() {
        out.extend(run_file(&root(), case.path()).iter().map(Found::shown));
    }
    out
}

/// Run every script under `dir` and fail naming each difference.
fn run(dir: &Path) {
    let options = Options::new(Sides::Engine, WORKERS);
    let scripts = runner::select(&corpus(), dir).cases().len();
    let passed = runner::passes(&corpus(), dir, &options).unwrap_or_else(|out| panic!("{out}"));
    assert_eq!(scripts, passed, "{}", dir.display());
}

#[test]
fn oplog() {
    run(&root().join("oplog"));
}

#[test]
fn bundles() {
    run(&root().join("bundles"));
}

#[test]
fn roles() {
    run(&root().join("roles"));
}

#[test]
fn authority() {
    run(&root().join("authority"));
}

#[test]
fn registered() {
    run(&root().join("registered"));
}

#[test]
fn registration() {
    run(&root().join("registration"));
}

#[test]
fn commands() {
    run(&root().join("commands"));
}

#[test]
fn api() {
    run(&root().join("api"));
}

#[test]
fn past() {
    run(&root().join("past"));
}

#[test]
fn longkey() {
    run(&root().join("longkey"));
}

/// Returns the override blocks of the nextest configuration.
fn overrides() -> Vec<String> {
    let config = crate_dir().join("../../.config/nextest.toml");
    let text = fs::read_to_string(&config).expect("the nextest configuration reads");
    text.split("[[profile.default.overrides]]")
        .skip(1)
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_area_has_its_test_holding_its_slots() {
    let source = include_str!("mod.rs");
    let blocks = overrides();
    let mut areas = 0;
    for entry in fs::read_dir(root()).expect("the scripts directory reads") {
        let path = entry.expect("a directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).expect("a name");
        if !path.is_dir() || ["fixtures", "probes"].contains(&name) {
            continue;
        }
        areas += 1;
        let test = format!("fn {name}() {{\n    run(&root().join(\"{name}\"));");
        assert!(source.contains(&test), "scripts/{name} has no test");
        let filter = format!("test(=scripts::{name})");
        let slots = format!("threads-required = {WORKERS}\n");
        assert!(
            blocks
                .iter()
                .any(|b| b.contains(&filter) && b.contains(&slots)),
            "no override in .config/nextest.toml gives `{filter}` {WORKERS} slots"
        );
    }
    assert!(areas > 0, "no area under {}", root().display());
}

#[test]
fn every_adtb_sim_override_names_the_binary_that_holds_its_tests() {
    const BINARY: &str = "package(=adtb-sim) and binary(=";
    let tests = crate_dir().join("tests");
    let mut checked = 0;
    for block in overrides() {
        let Some((_, rest)) = block.split_once(BINARY) else {
            continue;
        };
        let (binary, _) = rest.split_once(')').expect("a closed binary term");
        let main = fs::read_to_string(tests.join(binary).join("main.rs"))
            .unwrap_or_else(|e| panic!("binary `{binary}` has no main.rs: {e}"));
        for (_, rest) in block
            .match_indices("test(=")
            .map(|(i, _)| block.split_at(i + 6))
        {
            let name = rest.split(')').next().unwrap_or_default();
            let Some((module, _)) = name.split_once("::") else {
                continue;
            };
            let declared = format!("mod {module};");
            assert!(
                main.lines().any(|line| line == declared),
                "override names `{module}::` in binary `{binary}`, which does not declare it"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no adtb-sim override names a test");
}

/// Returns the expected differences of probe `path`: the lines of the file beside it named
/// with `.differs`.
fn expected(path: &Path) -> Vec<String> {
    let file = path.with_extension("differs");
    let text = fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("{} does not read: {e}", file.display()));
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match out.last_mut() {
            Some(last) if line.starts_with("  ") => {
                last.push('\n');
                last.push_str(line);
            }
            _ => out.push(line.to_owned()),
        }
    }
    out
}

#[test]
fn each_probe_differs_as_it_states() {
    let probes = root().join("probes");
    let mut ran = 0;
    for entry in fs::read_dir(&probes).expect("the probes directory reads") {
        let path = entry.expect("a directory entry").path();
        let got = match path.extension().and_then(|x| x.to_str()) {
            Some("test") => run_file(&root(), &path).iter().map(Found::shown).collect(),
            Some("differs") => continue,
            _ if path.is_dir() => differences(&path),
            _ => panic!("{} is no probe", path.display()),
        };
        assert_eq!(expected(&path), got, "{}", path.display());
        ran += 1;
    }
    assert!(ran > 0, "no probes under {}", probes.display());
}

#[test]
fn an_area_that_does_not_read_or_holds_no_script_fails() {
    let scratch = Scratch::new("scripts", "unreadable-area");
    let dir = scratch.to_path_buf();
    let locked = dir.join("locked");
    fs::create_dir_all(&locked).expect("the directory is created");
    let mode = |m| {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(m)).expect("the mode is set");
    };
    mode(0o000);
    let got = differences(&dir);
    mode(0o755);
    let shown = dir.display();
    assert_eq!(
        vec![
            format!("{shown}/locked: Permission denied (os error 13)"),
            format!("no cases under {shown}"),
        ],
        got
    );
}
