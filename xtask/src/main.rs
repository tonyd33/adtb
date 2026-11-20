//! `cargo xtask test [NEXTEST ARGS]`, `cargo xtask lint` and `cargo xtask check [REVSET]`: the
//! workspace's test run under a memory limit, its TQL rules and its full set of checks.

#[cfg(test)]
extern crate adtb_heap_cap as _;

mod check;
mod grammar;
mod lint;
mod scope;
mod size;
mod step;
mod test;
mod tree;
mod workspace;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cargo xtask",
    bin_name = "cargo xtask",
    about = "Test and check the adtb workspace."
)]
struct Args {
    #[command(subcommand)]
    task: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Run `cargo nextest run --no-fail-fast ARGS` over the workspace in a memory-limited scope.
    ///
    /// The full output goes to `target/test.txt`. Prints the build errors, the summary with every
    /// failed, killed and timed out test, the scope's peak memory and that path, and fails when
    /// nextest does.
    ///
    /// The scope is a systemd user scope of its own under `adtb.slice`, holding at most 14G.
    /// `adtb.slice`, every run together, is throttled past 60% of the machine's memory and holds
    /// at most 75%. Neither swaps. Past a limit the kernel kills a process under it, nextest
    /// reports it as SIGKILL and the run goes on. With no systemd user manager the run is
    /// unlimited, and says so.
    Test {
        /// Arguments to `cargo nextest run`.
        #[arg(
            value_name = "NEXTEST ARGS",
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        args: Vec<OsString>,
    },
    /// Run every check of the workspace in one memory-limited scope, as `test` does.
    ///
    /// In a scope of its own on at least 8 cores, run the tests and the damage profile at once, a
    /// quarter of the test threads to the damage profile, and beside them clippy, building into
    /// `clippy` under the target directory; when clippy fails, kill the tests and the damage
    /// profile and report them stopped. Otherwise run clippy, then, once it passes, the tests, then
    /// the damage profile. Beside them run rustfmt, the TQL rules, cargo-deny, the source-size
    /// limit, the documentation build and its link check, the editor grammar's generated sources
    /// and its corpus and highlight tests, the Loom models and Miri.
    /// Each stage's output goes to `check/STAGE.log` under the target directory. Print each
    /// stage's wall time as it ends, with the whole output of one that failed, and fail when any
    /// stage fails. Miri needs `ADTB_NIGHTLY_BIN`, which the dev shell sets.
    Check {
        /// Also check the lint allowlist against that of the parent of the revset's root.
        #[arg(value_name = "REVSET")]
        revset: Option<String>,
    },
    /// Run one stage of `check` in this process.
    #[command(hide = true)]
    Stage {
        #[arg(value_parser = check::Stage::parse)]
        stage: check::Stage,
        /// Check the lint allowlist against the one at this revision.
        #[arg(long, value_name = "REV")]
        base: Option<String>,
        /// Run the tests or the damage search on this many test threads.
        #[arg(long, value_name = "N")]
        test_threads: Option<usize>,
        /// Build clippy into `clippy` under the target directory.
        #[arg(long)]
        own_target: bool,
    },
    /// Run the TQL rules `lint/rules.toml` declares, with `tql` from PATH.
    ///
    /// Print `FILE:LINE: ROW` for each site of a zero row, and each file whose count of a
    /// ratchet row differs from its entry in `lint/allowlist.toml`, a missing entry counting as
    /// 0. Fails when it prints.
    Lint {
        /// Also fail when an entry of the allowlist is above the one of the allowlist at REV.
        #[arg(long, value_name = "REV")]
        against: Option<String>,
        /// Print the allowlist for the tree instead of checking it.
        #[arg(long, conflicts_with = "against")]
        counts: bool,
    },
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => return usage(&e),
    };
    let result = zero_core_limit().and_then(|()| match args.task {
        Task::Test { args } => {
            scope::enter("test").and_then(|()| test::run(&workspace::Workspace::find()?, &args))
        }
        Task::Check { revset } => scope::enter("check")
            .and_then(|()| check::run(&workspace::Workspace::find()?, revset.as_deref())),
        Task::Stage {
            stage,
            base,
            test_threads,
            own_target,
        } => check::stage(
            &workspace::Workspace::find()?,
            stage,
            base.as_deref(),
            test_threads,
            own_target,
        ),
        Task::Lint { against, counts } => lint::run(
            &workspace::Workspace::find()?.root,
            against.as_deref(),
            counts,
        ),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => match writeln!(std::io::stderr(), "xtask: {e}") {
            Ok(()) => ExitCode::FAILURE,
            Err(_) => ExitCode::from(2),
        },
    }
}

/// Print `e`. Returns 2 for a usage error and 0 for help.
fn usage(e: &clap::Error) -> ExitCode {
    if e.print().is_err() || e.use_stderr() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// Set this process's soft `RLIMIT_CORE` to zero and keep its hard limit, for every command it
/// starts.
fn zero_core_limit() -> Result<(), String> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: the pointer names an `rlimit` the call fills.
    if unsafe { libc::getrlimit(libc::RLIMIT_CORE, &raw mut limit) } != 0 {
        return Err(format!(
            "core limit: getrlimit failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    limit.rlim_cur = 0;
    // SAFETY: the pointer names an initialised `rlimit`.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raw const limit) } != 0 {
        return Err(format!(
            "core limit: setrlimit failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
