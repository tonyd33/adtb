use std::fmt::Display;
use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

/// Write `line` to standard error.
pub fn say(line: impl Display) -> Result<(), String> {
    writeln!(std::io::stderr(), "{line}").map_err(|e| format!("writing to standard error: {e}"))
}

/// Run `command` with the standard streams of this process.
pub fn run(step: &str, command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("{step}: {} did not start: {e}", shown(command)))?;
    succeeded(step, command, status)
}

/// Run `command` and return its standard output; its standard error passes through.
pub fn output(step: &str, command: &mut Command) -> Result<Vec<u8>, String> {
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("{step}: {} did not start: {e}", shown(command)))?;
    succeeded(step, command, output.status)?;
    Ok(output.stdout)
}

pub fn succeeded(step: &str, command: &Command, status: ExitStatus) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{step} failed: {} {status}", shown(command)))
    }
}

/// Returns `command` as the variables other than `PATH` it sets, its program and its arguments,
/// each quoted as Rust quotes a string.
pub fn shown(command: &Command) -> String {
    let variables = command.get_envs().filter_map(|(name, value)| {
        value
            .filter(|_| name != "PATH")
            .map(|value| format!("{}={:?}", name.to_string_lossy(), value.to_string_lossy()))
    });
    let words = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|word| format!("{:?}", word.to_string_lossy()));
    variables.chain(words).collect::<Vec<_>>().join(" ")
}
