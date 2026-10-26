use adtb::storage::Pager;
use adtb::verify::{Inventory, repair, verify};
use adtb_sim::SimRuntime;

/// Returns what `verify` finds wrong with the file after a crash: every finding `repair` does not
/// mend, then, once it has mended the rest, every finding of a second pass.
pub fn verify_after_crash(pager: &Pager<SimRuntime>, inventory: &Inventory) -> Vec<String> {
    let report = verify(pager, inventory);
    let mut problems: Vec<String> = report
        .unrepairable()
        .iter()
        .map(|f| format!("{f:?}"))
        .collect();
    problems.extend(report.log.iter().map(|l| format!("{l:?}")));
    if !problems.is_empty() {
        return problems;
    }
    repair(pager, inventory, &report).expect("repair");
    let again = verify(pager, inventory);
    let mut problems: Vec<String> = again
        .findings
        .iter()
        .map(|f| format!("after repair: {f:?}"))
        .collect();
    problems.extend(again.log.iter().map(|l| format!("after repair: {l:?}")));
    problems
}
