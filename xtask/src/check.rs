use std::ffi::OsString;
use std::path::PathBuf;

use crate::step::{self, say};
use crate::workspace::Workspace;
use crate::{grammar, lint, size, test};

/// The variable naming the directory of a nightly toolchain with Miri.
const NIGHTLY: &str = "ADTB_NIGHTLY_BIN";

/// Run every check of `workspace` in order, stopping at the first that fails. With `revset`, also
/// check the lint allowlist against that of its base.
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
    stage("clippy", |name| {
        step::run(
            name,
            workspace.cargo(&["clippy"]).args([
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
        )
    })?;
    stage("fmt", |name| {
        step::run(name, workspace.cargo(&["fmt"]).args(["--all", "--check"]))
    })?;
    stage(lint::STEP, |_| {
        lint::run(&workspace.root, base.as_deref(), false)
    })?;
    stage(test::STEP, |_| test::run(workspace, &[]))?;
    stage("loom", |name| loom(workspace, name))?;
    stage("miri", |name| miri(workspace, name))?;
    stage("deny", |name| {
        step::run(name, workspace.cargo(&["deny"]).arg("check"))
    })?;
    stage("damage", |name| {
        step::run(
            name,
            workspace.cargo(&["nextest", "run"]).args([
                "-p",
                "adtb-sim",
                "--test",
                "storage",
                "--profile",
                "damage",
            ]),
        )
    })?;
    stage(size::STEP, |_| size::run(&workspace.root))?;
    stage("docs", |name| docs(workspace, name))?;
    stage(grammar::STEP, |_| grammar::run(workspace))?;
    Ok(())
}

/// Announce the step `name`, then run `body` with it.
fn stage(name: &str, body: impl FnOnce(&str) -> Result<(), String>) -> Result<(), String> {
    say(format_args!("xtask check: {name}"))?;
    body(name)
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
