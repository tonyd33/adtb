//! Verify a pager's file after a simulated crash.

use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::verify::{Inventory, Problem, verify};

/// Returns what `verify` finds wrong with the file after a crash: every finding but a leaked page,
/// which an open frees, and what the mend pass mends.
pub fn verify_after_crash<R: Runtime>(pager: &Pager<R>, inventory: &Inventory) -> Vec<String> {
    let report = verify(pager, inventory);
    let mut problems: Vec<String> = report
        .unmended()
        .iter()
        .filter(|f| f.problem != Problem::Leaked)
        .map(|f| format!("{f:?}"))
        .collect();
    problems.extend(report.log.iter().map(|l| format!("{l:?}")));
    problems
}
