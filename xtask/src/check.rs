use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::step::{self, say};
use crate::tree::{self, Pidfd};
use crate::workspace::Workspace;
use crate::{grammar, lint, repo, scope, size, test};

/// The variable naming the directory of a nightly toolchain with Miri.
const NIGHTLY: &str = "ADTB_NIGHTLY_BIN";

/// A stage of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fmt,
    Lint,
    Deny,
    Size,
    Repo,
    Docs,
    TreeSitter,
    Loom,
    Miri,
    Clippy,
    Build,
    Tests,
    Damage,
}

/// The stages that share no build with the tests, each run at once.
const FREE: [Stage; 9] = [
    Stage::Fmt,
    Stage::Lint,
    Stage::Deny,
    Stage::Size,
    Stage::Repo,
    Stage::Docs,
    Stage::TreeSitter,
    Stage::Loom,
    Stage::Miri,
];

/// The stages that run the build stage's test binaries once it passes.
const TESTS: [Stage; 2] = [Stage::Tests, Stage::Damage];

/// The stages that build in or run from the workspace's target directory. When they run beside
/// clippy they stop when it fails; otherwise they run after it passes.
const BUILT: [Stage; 3] = [Stage::Build, Stage::Tests, Stage::Damage];

/// The flag of `xtask stage clippy` that builds into `clippy` under the target directory.
const OWN_TARGET: &str = "--own-target";

/// The nextest slots each damage search test holds.
const DAMAGE_SLOTS: usize = 4;

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Lint => lint::STEP,
            Self::Deny => "deny",
            Self::Size => size::STEP,
            Self::Repo => repo::STEP,
            Self::Docs => "docs",
            Self::TreeSitter => grammar::STEP,
            Self::Loom => "loom",
            Self::Miri => "miri",
            Self::Clippy => "clippy",
            Self::Build => "build",
            Self::Tests => test::STEP,
            Self::Damage => "damage",
        }
    }

    /// Returns the stage named `name`.
    pub fn parse(name: &str) -> Result<Self, String> {
        FREE.into_iter()
            .chain([Self::Clippy])
            .chain(BUILT)
            .find(|stage| stage.name() == name)
            .ok_or_else(|| format!("no stage {name}"))
    }
}

/// Run every stage of the check, each a process of this program with its output in
/// `check/STAGE.log` under the target directory, and print each stage's result as it ends with the
/// whole output of one that failed. With `revset`, also check the lint allowlist against that of its base. Fails when any stage fails.
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
    #[expect(
        clippy::disallowed_methods,
        reason = "goes if the check stops recording its wall time"
    )]
    let started = Instant::now();
    let logs = workspace.target.join("check");
    let times = logs.join("times.tsv");
    std::fs::create_dir_all(&logs)
        .map_err(|e| format!("xtask check: creating {}: {e}", logs.display()))?;
    let suite = workspace.target.join("test.txt");
    std::fs::write(&suite, "xtask check: the suite did not run\n")
        .map_err(|e| format!("xtask check: writing {}: {e}", suite.display()))?;
    let program =
        std::env::current_exe().map_err(|e| format!("xtask check: no path to xtask: {e}"))?;
    tree::adopt_orphans().map_err(|e| format!("xtask check: {e}"))?;
    let launch = Launch {
        program,
        logs,
        base,
        children: Mutex::default(),
    };
    let (sender, ended) = mpsc::channel();
    let failed = thread::scope(|s| {
        let launch = &launch;
        for stage in FREE {
            let sender = sender.clone();
            s.spawn(move || launch.run(stage, &[]).map(|ran| sender.send(ran)));
        }
        match overlap() {
            Some(damage) => {
                let sender_clippy = sender.clone();
                s.spawn(move || {
                    launch
                        .run(Stage::Clippy, &[OWN_TARGET.to_owned()])
                        .map(|ran| sender_clippy.send(ran))
                });
                let sender = sender.clone();
                s.spawn(move || {
                    let build = launch.run(Stage::Build, &[])?;
                    let passed = build.passed();
                    sender.send(build).ok()?;
                    if passed {
                        let damage = vec!["--test-threads".to_owned(), damage.to_string()];
                        for (stage, threads) in TESTS.into_iter().zip([Vec::new(), damage]) {
                            let sender = sender.clone();
                            s.spawn(move || {
                                launch.run(stage, &threads).map(|ran| sender.send(ran))
                            });
                        }
                    }
                    Some(())
                });
            }
            None => {
                let sender = sender.clone();
                s.spawn(move || {
                    for stage in [Stage::Clippy, Stage::Build] {
                        let ran = launch.run(stage, &[])?;
                        let passed = ran.passed();
                        sender.send(ran).ok()?;
                        if !passed {
                            return Some(());
                        }
                    }
                    for stage in TESTS {
                        sender.send(launch.run(stage, &[])?).ok()?;
                    }
                    Some(())
                });
            }
        }
        drop(sender);
        let mut ended_stages = Vec::new();
        let mut failed = Vec::new();
        let mut rows = String::new();
        for ran in ended {
            ran.report()?;
            rows.push_str(&format!(
                "{}\t{}\t{:.1}\n",
                ran.stage.name(),
                ran.word(),
                ran.took.as_secs_f64()
            ));
            if ran.stage == Stage::Clippy && !ran.passed() {
                launch.stop()?;
            }
            if ran.stage == Stage::Tests && matches!(ran.outcome, Outcome::Stopped) {
                mark_stopped(&workspace.target.join("test.txt"))?;
            }
            if ran.failed() {
                failed.push(ran.stage.name());
            }
            ended_stages.push(ran.stage);
        }
        for stage in BUILT.into_iter().filter(|s| !ended_stages.contains(s)) {
            say(format_args!("xtask check: {} did not run", stage.name()))?;
            rows.push_str(&format!("{}\tdid not run\t\n", stage.name()));
        }
        rows.push_str(&format!(
            "total\t\t{:.1}\n",
            started.elapsed().as_secs_f64()
        ));
        std::fs::write(&times, rows)
            .map_err(|e| format!("xtask check: writing {}: {e}", times.display()))?;
        Ok::<_, String>(failed)
    })?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("check failed in {}", failed.join(", ")))
    }
}

/// Returns the test threads of the damage search when it runs at once with the suite: in a scope
/// `cargo xtask` made, on at least twice the damage search's slots of cores. The damage search
/// gets a quarter of the cores and at least its slots; the suite keeps a thread for every core.
/// Returns `None` otherwise, and the damage search runs after the suite.
fn overlap() -> Option<usize> {
    scope::own_scope()?;
    let cores = thread::available_parallelism().ok()?.get();
    (cores >= 2 * DAMAGE_SLOTS).then(|| (cores / 4).max(DAMAGE_SLOTS))
}

/// Run `stage` of `workspace` with the standard streams of this process. With `base`, the lint
/// stage also checks the allowlist against the one at that revision. With `threads`, the tests
/// and the damage search run that many test threads. With `own_target`, clippy builds into
/// `clippy` under the target directory.
pub fn stage(
    workspace: &Workspace,
    stage: Stage,
    base: Option<&str>,
    threads: Option<usize>,
    own_target: bool,
) -> Result<(), String> {
    let name = stage.name();
    let threads: Vec<OsString> = threads
        .map(|n| vec!["--test-threads".into(), n.to_string().into()])
        .unwrap_or_default();
    match stage {
        Stage::Clippy => {
            let clippy = |args: &[String]| {
                let mut clippy = workspace.cargo(&["clippy"]);
                clippy.args(args).args(["--", "-D", "warnings"]);
                if own_target {
                    clippy.env("CARGO_TARGET_DIR", workspace.target.join("clippy"));
                }
                clippy
            };
            let all = ["--workspace", "--all-targets"].map(str::to_owned);
            step::run(name, &mut clippy(&all))?;
            let shipped: Vec<String> = shipped(workspace)?
                .into_iter()
                .flat_map(|package| ["-p".to_owned(), package])
                .collect();
            step::run(name, &mut clippy(&shipped))
        }
        Stage::Fmt => step::run(name, workspace.fmt().args(["--all", "--check"])),
        Stage::Lint => lint::run(&workspace.root, base, false),
        Stage::Build => test::build(workspace).and_then(|output| {
            std::io::stderr()
                .write_all(&output)
                .map_err(|e| format!("{name}: writing cargo's output: {e}"))
        }),
        Stage::Tests => test::run_built(workspace, &threads),
        Stage::Loom => loom(workspace, name),
        Stage::Miri => miri(workspace, name),
        Stage::Deny => step::run(name, workspace.cargo(&["deny"]).arg("check")),
        Stage::Damage => step::run(
            name,
            test::reuse(workspace)
                .args(["--profile", "damage"])
                .args(&threads),
        ),
        Stage::Size => size::run(&workspace.root),
        Stage::Repo => repo::run(&workspace.root),
        Stage::Docs => docs(workspace, name),
        Stage::TreeSitter => grammar::run(workspace),
    }
}

/// How a stage starts: this program, the directory of the stages' logs, the allowlist base and the
/// stages running.
struct Launch {
    program: PathBuf,
    logs: PathBuf,
    base: Option<String>,
    children: Mutex<Children>,
}

/// The stages running, each a child of this process not yet reaped, and what `stop` did.
#[derive(Default)]
struct Children {
    running: Vec<(Stage, Pidfd)>,
    stopped: bool,
    killed: Vec<Stage>,
}

/// A stage that ended: its name, its log, its wall time and how it ended.
struct Ran {
    stage: Stage,
    log: PathBuf,
    took: Duration,
    outcome: Outcome,
}

enum Outcome {
    Exited(ExitStatus),
    /// Killed after clippy failed.
    Stopped,
    /// Did not start, or could not be waited for.
    Lost(String),
}

impl Launch {
    /// Run `stage` in a process of its own with `args`, its output in its log. Returns `None` for
    /// the build, the tests or the damage search after clippy failed.
    #[expect(
        clippy::disallowed_methods,
        reason = "goes if the check stops reporting each stage's wall time"
    )]
    fn run(&self, stage: Stage, args: &[String]) -> Option<Ran> {
        let log = self.logs.join(format!("{}.log", stage.name()));
        let started = Instant::now();
        let outcome = match self.start(stage, args, &log) {
            Ok(Some((child, exit))) => self.wait(stage, child, &exit),
            Ok(None) => return None,
            Err(e) => Outcome::Lost(e),
        };
        Some(Ran {
            stage,
            log,
            took: started.elapsed(),
            outcome,
        })
    }

    /// Start `stage`, and return it with a second handle on it to wait for its exit.
    fn start(
        &self,
        stage: Stage,
        args: &[String],
        log: &Path,
    ) -> Result<Option<(Child, Pidfd)>, String> {
        let out = File::create(log).map_err(|e| format!("creating {}: {e}", log.display()))?;
        let err = out
            .try_clone()
            .map_err(|e| format!("sharing {}: {e}", log.display()))?;
        let mut command = Command::new(&self.program);
        command.args(["stage", stage.name()]);
        if let Some(base) = &self.base {
            command.args(["--base", base]);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        let mut children = self.children();
        if BUILT.contains(&stage) && children.stopped {
            return Ok(None);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} did not start: {e}", step::shown(&command)))?;
        match Pidfd::open(child.id()).and_then(|held| Ok((held, Pidfd::open(child.id())?))) {
            Ok((held, exit)) => {
                children.running.push((stage, held));
                Ok(Some((child, exit)))
            }
            Err(e) => {
                drop(children);
                let waited = child.wait();
                Err(format!("{e}; the stage then ended: {waited:?}"))
            }
        }
    }

    /// Wait for `stage` to exit, take it from the running stages, then reap it. A stage is reaped
    /// only once taken, so `stop` signals no reaped pid.
    fn wait(&self, stage: Stage, mut child: Child, exit: &Pidfd) -> Outcome {
        let exited = exit.exited(-1);
        let killed = {
            let mut children = self.children();
            children.running.retain(|(running, _)| *running != stage);
            children.killed.contains(&stage)
        };
        match (exited, child.wait()) {
            (_, Ok(status)) if killed && !status.success() => Outcome::Stopped,
            (Ok(_), Ok(status)) => Outcome::Exited(status),
            (Err(e), Ok(status)) => Outcome::Lost(format!("{e}; it then ended: {status}")),
            (_, Err(e)) => Outcome::Lost(format!("waiting for it: {e}")),
        }
    }

    /// Kill the build, the tests and the damage search with every process under them and start none,
    /// then print any process under the check that is still running and is no stage.
    fn stop(&self) -> Result<(), String> {
        let mut children = self.children();
        children.stopped = true;
        let mut killed = Vec::new();
        for (stage, held) in children.running.iter().filter(|(s, _)| BUILT.contains(s)) {
            killed.push(*stage);
            let left = tree::kill(held).map_err(|e| format!("xtask check: stopping: {e}"))?;
            for pid in left {
                say(format_args!(
                    "xtask check: process {pid} under {} survived the kill",
                    stage.name()
                ))?;
            }
        }
        children.killed.extend(killed);
        let stages: Vec<u32> = children.running.iter().map(|(_, held)| held.pid).collect();
        for (pid, name) in tree::adopted().map_err(|e| format!("xtask check: {e}"))? {
            if !stages.contains(&pid) {
                say(format_args!(
                    "xtask check: process {pid} ({name}) runs under the check outside any stage"
                ))?;
            }
        }
        Ok(())
    }

    fn children(&self) -> MutexGuard<'_, Children> {
        self.children.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Append to the tests' output in `out` that the run was stopped.
fn mark_stopped(out: &Path) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(out)
        .and_then(|mut file| {
            writeln!(
                file,
                "xtask test: stopped, since clippy failed; this output is partial"
            )
        })
        .map_err(|e| format!("xtask check: marking {}: {e}", out.display()))
}

impl Ran {
    fn passed(&self) -> bool {
        matches!(&self.outcome, Outcome::Exited(status) if status.success())
    }

    fn failed(&self) -> bool {
        !self.passed() && !matches!(self.outcome, Outcome::Stopped)
    }

    fn word(&self) -> &'static str {
        match self.outcome {
            Outcome::Exited(status) if status.success() => "passed",
            Outcome::Stopped => "stopped",
            Outcome::Exited(_) | Outcome::Lost(_) => "failed",
        }
    }

    /// Print the stage's result and wall time, after its whole output when it failed.
    fn report(&self) -> Result<(), String> {
        let (name, secs) = (self.stage.name(), self.took.as_secs_f64());
        let why = match &self.outcome {
            Outcome::Exited(status) if status.success() => {
                return say(format_args!("xtask check: {name} passed in {secs:.1} s"));
            }
            Outcome::Stopped => {
                return say(format_args!(
                    "xtask check: {name} stopped after {secs:.1} s, since clippy failed"
                ));
            }
            Outcome::Exited(status) => status.to_string(),
            Outcome::Lost(e) => e.clone(),
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
    let mut miri = workspace.cargo_of(&cargo, &["miri", "test"]);
    miri.args(["-p", "adtb-sys", "--lib"])
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

/// Returns the workspace members whose manifest has no `test-support = true` under
/// `[package.metadata.adtb]`.
fn shipped(workspace: &Workspace) -> Result<Vec<String>, String> {
    let name = "shipped crates";
    let metadata = step::output(
        name,
        workspace
            .cargo(&["metadata"])
            .args(["--format-version", "1", "--no-deps"]),
    )?;
    let metadata: serde_json::Value = serde_json::from_slice(&metadata)
        .map_err(|e| format!("{name}: cargo metadata printed no JSON: {e}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| format!("{name}: cargo metadata printed no `packages`"))?;
    packages
        .iter()
        .filter(|package| package["metadata"]["adtb"]["test-support"] != true)
        .map(|package| {
            package["name"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{name}: a package with no name"))
        })
        .collect()
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
