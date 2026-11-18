use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::step;

const SLICE: &str = "adtb.slice";
const UNIT_PREFIX: &str = "adtb-run-";

/// Re-execute this process in a systemd user scope of its own under `adtb.slice`, unless it
/// already runs in such a scope, or there is no cgroup v2 path or systemd user manager. Returns
/// only in those cases or on an error.
pub fn enter(task: &str) -> Result<(), String> {
    match cgroup() {
        Some(cgroup) if is_run_scope(&cgroup) => return Ok(()),
        Some(_) if has_user_manager() => {}
        _ => {
            return step::say(format_args!(
                "xtask: no cgroup v2 path or systemd user manager, so `xtask {task}` runs \
                 without a memory limit"
            ));
        }
    }
    step::run(
        "memory limit",
        Command::new("systemctl").args([
            "--user",
            "set-property",
            "--runtime",
            SLICE,
            "MemoryHigh=60%",
            "MemoryMax=75%",
            "MemorySwapMax=0",
        ]),
    )?;
    let program =
        std::env::current_exe().map_err(|e| format!("memory limit: no path to xtask: {e}"))?;
    let mut scope = Command::new("systemd-run");
    scope
        .args(["--user", "--scope", "--quiet", "--collect"])
        .arg(format!("--slice={SLICE}"))
        .arg(format!(
            "--unit={UNIT_PREFIX}xtask-{task}-{}",
            std::process::id()
        ))
        .args([
            "-p",
            "MemoryMax=14G",
            "-p",
            "MemorySwapMax=0",
            "-p",
            "OOMPolicy=continue",
        ])
        .arg("--")
        .arg(program)
        .args(std::env::args_os().skip(1));
    let error = scope.exec();
    Err(format!(
        "memory limit: {} did not start: {error}",
        step::shown(&scope)
    ))
}

fn has_user_manager() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Returns the cgroup v2 path of this process, or `None` when `/proc/self/cgroup` has none.
fn cgroup() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::to_owned)
}

fn is_run_scope(cgroup: &str) -> bool {
    let unit = cgroup.rsplit('/').next().unwrap_or_default();
    unit.starts_with(UNIT_PREFIX) && unit.ends_with(".scope")
}

/// Returns the cgroup of this process when it is a scope `enter` made.
pub fn own_scope() -> Option<String> {
    cgroup().filter(|cgroup| is_run_scope(cgroup))
}

/// Returns the peak memory of `cgroup` in MiB.
pub fn peak_mib(cgroup: &str) -> Result<u64, String> {
    let file = format!("/sys/fs/cgroup{cgroup}/memory.peak");
    let text =
        std::fs::read_to_string(&file).map_err(|e| format!("memory limit: reading {file}: {e}"))?;
    text.trim()
        .parse::<u64>()
        .map(|bytes| bytes >> 20)
        .map_err(|e| format!("memory limit: {file} holds no byte count: {e}"))
}
