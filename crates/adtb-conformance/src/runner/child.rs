//! Run one conformance file on the engine in a child process, so a file that overflows the
//! stack, aborts or does not finish fails alone.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::Found;

use super::{Case, shown};

const DEADLINE: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(20);

/// Returns `found` as the one line a child prints.
pub fn encode(found: &[Found]) -> String {
    let each = |f: &Found| {
        json!({
            "line": f.line,
            "head": f.head,
            "values": f.values,
            "rows": f.rows,
        })
    };
    Json::Array(found.iter().map(each).collect()).to_string()
}

/// Returns the differences a child printed with [`encode`], or why they do not read.
pub fn decode(text: &str) -> Result<Vec<Found>, String> {
    let all: Json = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let strings = |v: &Json| -> Option<Vec<String>> {
        v.as_array()?
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    let pair = |v: &Json| -> Option<Option<(Json, Json)>> {
        match v {
            Json::Null => Some(None),
            Json::Array(p) if p.len() == 2 => Some(Some((p[0].clone(), p[1].clone()))),
            _ => None,
        }
    };
    let one = |f: &Json| -> Option<Found> {
        let line = match &f["line"] {
            Json::Null => None,
            n => Some(usize::try_from(n.as_u64()?).ok()?),
        };
        let values = match pair(&f["values"])? {
            Some((e, a)) => Some((e.as_str()?.to_owned(), a.as_str()?.to_owned())),
            None => None,
        };
        let rows = match pair(&f["rows"])? {
            Some((e, a)) => Some((strings(&e)?, strings(&a)?)),
            None => None,
        };
        Some(Found {
            line,
            head: f["head"].as_str()?.to_owned(),
            values,
            rows,
        })
    };
    all.as_array()
        .ok_or("not an array")?
        .iter()
        .map(|f| one(f).ok_or_else(|| format!("not a difference: {f}")))
        .collect()
}

/// Returns each difference of the conformance file `case` on the engine, run by `program` in a
/// child process.
pub(super) fn on_engine(program: &Path, case: &Case) -> Vec<Found> {
    let at = shown(&case.root, &case.path);
    let failed = |message: String| vec![Found::at(None, format!("{at}:0: engine: {message}"))];
    let spawned = Command::new(program)
        .arg("engine-case")
        .arg(&case.root)
        .arg(&case.path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return failed(format!("{} does not start: {e}", program.display())),
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let drain = |r: &mut dyn Read| {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes).map(|_| bytes)
    };
    let (status, out, err) = thread::scope(|s| {
        let out = s.spawn(|| drain(&mut stdout));
        let err = s.spawn(|| drain(&mut stderr));
        let mut status = None;
        for _ in 0..DEADLINE.as_millis() / POLL.as_millis() {
            status = child.try_wait().expect("the child is waited on");
            if status.is_some() {
                break;
            }
            thread::sleep(POLL);
        }
        if status.is_none() {
            child.kill().expect("the child is killed");
            child.wait().expect("the killed child is reaped");
        }
        let joined = |h: thread::ScopedJoinHandle<'_, std::io::Result<Vec<u8>>>| {
            h.join().expect("a pipe reader does not panic")
        };
        (status, joined(out), joined(err))
    });
    let Some(status) = status else {
        return failed(format!("does not finish within {DEADLINE:?}"));
    };
    let err = String::from_utf8_lossy(err.as_deref().unwrap_or_default()).into_owned();
    let tail = || {
        let lines: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(3)..].join(" / ")
    };
    let out = match out {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => return failed(format!("its output does not read: {e}")),
    };
    if !status.success() || out.trim().is_empty() {
        return failed(format!("aborts ({status}): {}", tail()));
    }
    decode(out.trim()).unwrap_or_else(|e| failed(format!("prints what does not read: {e}")))
}
