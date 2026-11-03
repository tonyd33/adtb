---
created-at: 2026-11-02
status: open
kind: bug
priority: 470
summary: a page whose LSN is past the recovered log end is reported by neither recovery nor verify, so a commit lost to a misdirected log write goes unseen
tags:
  - quality
  - engine
---

# Page past log end unreported

## Report

Found by the audit-floors track, run on main 6ab50187: `misdirected_and_lost_writes_are_reported_never_returned` in crates/adtb-sim/tests/pager.rs fails under `--profile deep` at seed 337, round 2, with `Lost(59)`. Default and quick seeds pass; seeds past 337 under deep are unchecked.

Trace: a log write is misdirected from @24576 to @22016, then a crash. The block holding transaction 59's commit never lands; the data page Pid(26) carrying 59's writes does. Recovery ends the log before 59's commit. Verify reports nothing: a page whose LSN is past the recovered log end is neither orphan (that needs `found < end`) nor stale. `judge` excuses a lost commit only on a log finding or a fault lost write on the log, not on a misdirect.

The contract the test holds: misdirected and lost writes are reported, never returned. Here an acknowledged commit is lost and nothing reports it.

Ruling: under write-ahead logging no page may carry an LSN past the durable log, so such a page is evidence that the log lost writes. Verify reports it as a log finding naming the page and both LSNs; `judge` keeps excusing a lost commit only on a finding. Do not excuse misdirects in `judge`. Whether recovery should refuse to open, or open and quarantine, on such a page is part of the fix: state the choice against docs/design/storage.md.

Done when the deep profile of that test passes over a seed range past 337, with a floor on the new finding.
