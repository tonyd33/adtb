use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::step::{self, say};
use crate::workspace::Workspace;
use crate::{grammar, lint, scope, size, test};

/// The variable naming the directory of a nightly toolchain with Miri.
const NIGHTLY: &str = "ADTB_NIGHTLY_BIN";

/// A stage of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fmt,
    Lint,
    Deny,
    Size,
    Docs,
    TreeSitter,
    Loom,
    Miri,
    Clippy,
    Tests,
    Damage,
}

/// The stages that build nothing in the workspace's target directory, each run at once.
const FREE: [Stage; 8] = [
    Stage::Fmt,
    Stage::Lint,
    Stage::Deny,
    Stage::Size,
    Stage::Docs,
    Stage::TreeSitter,
    Stage::Loom,
    Stage::Miri,
];

/// The stages that build in the workspace's target directory and run once clippy passes, beside
/// the free ones.
const AFTER_CLIPPY: [Stage; 2] = [Stage::Tests, Stage::Damage];

/// The nextest slots each damage search test holds.
const DAMAGE_SLOTS: usize = 4;

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Lint => lint::STEP,
            Self::Deny => "deny",
            Self::Size => size::STEP,
            Self::Docs => "docs",
            Self::TreeSitter => grammar::STEP,
            Self::Loom => "loom",
            Self::Miri => "miri",
            Self::Clippy => "clippy",
            Self::Tests => test::STEP,
            Self::Damage => "damage",
        }
    }

    /// Returns the stage named `name`.
    pub fn parse(name: &str) -> Result<Self, String> {
        FREE.into_iter()
            .chain([Self::Clippy])
            .chain(AFTER_CLIPPY)
            .find(|stage| stage.name() == name)
            .ok_or_else(|| format!("no stage {name}"))
    }
}

/// Run every stage of the check, each a process of this program with its output in
/// `check/STAGE.log` under the target directory, and print each stage's result as it ends with the
/// whole output of one that failed. With `revset`, also check the lint allowlist against that of
/// its base. Fails when any stage fails.
pub fn run(workspace: &Workspace, revset: Option<&str>) -> Result<(), String> {
    let base = match revset {
        Some(revset) => {
            let base = base(workspace, revset)?;
            if base.is_none() {
                say(format_args!(
                    "xtask check: no allowlist base: {revset} has no parent commit"
                ))?;
            }
            base
        }
        None => None,
    };
    let logs = workspace.target.join("check");
    std::fs::create_dir_all(&logs)
        .map_err(|e| format!("xtask check: creating {}: {e}", logs.display()))?;
    let program =
        std::env::current_exe().map_err(|e| format!("xtask check: no path to xtask: {e}"))?;
    let launch = Launch {
        program,
        logs,
        base,
    };
    let (sender, ended) = mpsc::channel();
    let failed = thread::scope(|s| {
        for stage in FREE {
            let sender = sender.clone();
            let launch = &launch;
            s.spawn(move || sender.send(launch.run(stage, None)));
        }
        let launch = &launch;
        s.spawn(move || {
            let clippy = launch.run(Stage::Clippy, None);
            let passed = clippy.passed();
            if sender.send(clippy).is_err() || !passed {
                return;
            }
            match overlap() {
                Some((suite, damage)) => {
                    for (stage, threads) in AFTER_CLIPPY.into_iter().zip([suite, damage]) {
                        let sender = sender.clone();
                        s.spawn(move || sender.send(launch.run(stage, Some(threads))));
                    }
                }
                None => {
                    for stage in AFTER_CLIPPY {
                        if sender.send(launch.run(stage, None)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let mut ended_stages = Vec::new();
        let mut failed = Vec::new();
        for ran in ended {
            ran.report()?;
            if !ran.passed() {
                failed.push(ran.stage.name());
            }
            ended_stages.push(ran.stage);
        }
        for stage in AFTER_CLIPPY
            .into_iter()
            .filter(|s| !ended_stages.contains(s))
        {
            say(format_args!("xtask check: {} did not run", stage.name()))?;
        }
        Ok::<_, String>(failed)
    })?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("check failed in {}", failed.join(", ")))
    }
}

/// Returns the test threads of the suite and of the damage search when they run at once: in a
/// scope `cargo xtask` made, on at least twice the damage search's slots of cores. The damage
/// search gets a quarter of the cores and at least its slots, the suite the rest. Returns `None`
/// otherwise, and the damage search runs after the suite.
fn overlap() -> Option<(usize, usize)> {
    scope::own_scope()?;
    let cores = thread::available_parallelism().ok()?.get();
    (cores >= 2 * DAMAGE_SLOTS).then(|| {
        let damage = (cores / 4).max(DAMAGE_SLOTS);
        (cores - damage, damage)
    })
}

/// Run `stage` of `workspace` with the standard streams of this process. With `base`, the lint
/// stage also checks the allowlist against the one at that revision. With `threads`, the tests
/// and the damage search run that many test threads.
pub fn stage(
    workspace: &Workspace,
    stage: Stage,
    base: Option<&str>,
    threads: Option<usize>,
) -> Result<(), String> {
    let name = stage.name();
    let threads: Vec<OsString> = threads
        .map(|n| vec!["--test-threads".into(), n.to_string().into()])
        .unwrap_or_default();
    match stage {
        Stage::Clippy => step::run(
            name,
            workspace.cargo(&["clippy"]).args([
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
        ),
        Stage::Fmt => step::run(name, workspace.cargo(&["fmt"]).args(["--all", "--check"])),
        Stage::Lint => lint::run(&workspace.root, base, false),
        Stage::Tests => test::run(workspace, &threads),
        Stage::Loom => loom(workspace, name),
        Stage::Miri => miri(workspace, name),
        Stage::Deny => step::run(name, workspace.cargo(&["deny"]).arg("check")),
        Stage::Damage => step::run(
            name,
            workspace
                .cargo(&["nextest", "run"])
                .args(["--workspace", "--profile", "damage"])
                .args(&threads),
        ),
        Stage::Size => size::run(&workspace.root),
        Stage::Docs => docs(workspace, name),
        Stage::TreeSitter => grammar::run(workspace),
    }
}

/// How a stage starts: this program, the directory of the stages' logs and the allowlist base.
struct Launch {
    program: PathBuf,
    logs: PathBuf,
    base: Option<String>,
}

/// A stage that ended: its name, its log, its wall time and its exit or why it did not start.
struct Ran {
    stage: Stage,
    log: PathBuf,
    took: Duration,
    status: Result<ExitStatus, String>,
}

impl Launch {
    /// Run `stage` in a process of its own, its output in its log, with `threads` test threads.
    #[expect(
        clippy::disallowed_methods,
        reason = "goes if the check stops reporting each stage's wall time"
    )]
    fn run(&self, stage: Stage, threads: Option<usize>) -> Ran {
        let name = stage.name();
        let log = self.logs.join(format!("{name}.log"));
        let started = Instant::now();
        let status = File::create(&log)
            .and_then(|file| Ok((file.try_clone()?, file)))
            .map_err(|e| format!("creating {}: {e}", log.display()))
            .and_then(|(out, err)| {
                let mut command = Command::new(&self.program);
                command.args(["stage", name]);
                if let Some(base) = &self.base {
                    command.args(["--base", base]);
                }
                if let Some(threads) = threads {
                    command.args(["--test-threads", &threads.to_string()]);
                }
                command
                    .stdin(Stdio::null())
                    .stdout(out)
                    .stderr(err)
                    .status()
                    .map_err(|e| format!("{} did not start: {e}", step::shown(&command)))
            });
        Ran {
            stage,
            log,
            took: started.elapsed(),
            status,
        }
    }
}

impl Ran {
    fn passed(&self) -> bool {
        self.status.as_ref().is_ok_and(ExitStatus::success)
    }

    /// Print the stage's result and wall time, after its whole output when it failed.
    fn report(&self) -> Result<(), String> {
        let (name, secs) = (self.stage.name(), self.took.as_secs_f64());
        let why = match &self.status {
            Ok(status) if status.success() => {
                return say(format_args!("xtask check: {name} passed in {secs:.1} s"));
            }
            Ok(status) => status.to_string(),
            Err(e) => e.clone(),
        };
        let output = std::fs::read(&self.log).map_or_else(
            |e| format!("reading {}: {e}\n", self.log.display()),
            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
        );
        say(format_args!(
            "xtask check: {name} failed in {secs:.1} s: {why}; its output:\n{}",
            output.trim_end()
        ))
    }
}

/// Lint adtb-sys under `--cfg adtb_loom`, then run the Loom models of the page state word, each
/// built into `loom` under the target directory.
fn loom(workspace: &Workspace, name: &str) -> Result<(), String> {
    let mut flags = OsString::from("--cfg adtb_loom");
    if let Some(outer) = std::env::var_os("RUSTFLAGS") {
        flags.push(" ");
        flags.push(outer);
    }
    let target = workspace.target.join("loom");
    let loom_cargo = |subcommand: &[&str]| {
        let mut cargo = workspace.cargo(subcommand);
        cargo
            .env("RUSTFLAGS", &flags)
            .env("CARGO_TARGET_DIR", &target);
        cargo
    };
    step::run(
        name,
        loom_cargo(&["clippy"]).args(["-p", "adtb-sys", "--all-targets", "--", "-D", "warnings"]),
    )?;
    step::run(
        name,
        loom_cargo(&["nextest", "run"]).args(["-p", "adtb-sys", "--lib", "pages::loom_tests::"]),
    )
}

/// Run adtb-sys's unit tests under Miri, with the nightly toolchain `ADTB_NIGHTLY_BIN` names.
fn miri(workspace: &Workspace, name: &str) -> Result<(), String> {
    let nightly = std::env::var_os(NIGHTLY)
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "{name}: {NIGHTLY} is unset; the dev shell sets it to a nightly toolchain with Miri"
            )
        })?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(nightly.clone()).chain(std::env::split_paths(&path)))
            .map_err(|e| format!("{name}: {NIGHTLY} cannot join PATH: {e}"))?;
    let cargo = nightly.join("cargo");
    let mut miri = workspace.command(&cargo);
    miri.args(["miri", "test", "--manifest-path"])
        .arg(workspace.manifest())
        .args(["-p", "adtb-sys", "--lib"])
        .env("PATH", path)
        .env("CARGO", &cargo);
    step::run(name, &mut miri)
}

/// Build the documentation site and check its links and fragments offline.
fn docs(workspace: &Workspace, name: &str) -> Result<(), String> {
    step::run(
        name,
        workspace
            .command("mdbook")
            .arg("build")
            .arg(&workspace.root),
    )?;
    let site = workspace.root.join("target/docs-site/html");
    step::run(
        name,
        workspace
            .command("lychee")
            .args([
                "--offline",
                "--include-fragments",
                "--no-progress",
                "--root-dir",
            ])
            .arg(&site)
            .arg(&site),
    )
}

/// Returns the parent of `revset`'s root.
fn base(workspace: &Workspace, revset: &str) -> Result<Option<String>, String> {
    let name = "allowlist base";
    let mut parents = workspace.command("jj");
    parents
        .arg("-R")
        .arg(&workspace.root)
        .args([
            "--ignore-working-copy",
            "--color=never",
            "log",
            "--no-graph",
            "-r",
        ])
        .arg(format!("roots({revset})-"))
        .args(["-T", r#"commit_id ++ "\n""#]);
    let parents = step::output(name, &mut parents)?;
    let parents =
        String::from_utf8(parents).map_err(|e| format!("{name}: jj printed no UTF-8: {e}"))?;
    Ok(parents.lines().next().map(str::to_owned))
}
