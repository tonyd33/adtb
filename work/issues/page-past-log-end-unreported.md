---
created-at: 2026-11-02
status: closed
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

## 2026-11-02: fix in review, workspace past-log-end

`LogProblem::Unlogged { pid, page }` from verify for a page whose LSN is past the log end recovery kept, and for a recovery Stale with found past expected. Only verify reports it; open neither refuses nor quarantines, since recovery reads only pages it redoes (docs/design/storage.md sections 3 and 7). Seed 337 passes under deep over 300..396. Review: no blocker, no false positive found on healthy databases, restore, live check or crash sweeps. Delta asked: a test reaching the no-record branch, one classification independent of open history with relation-scoped VERIFY keeping the finding, the known-damage path tested, docs.

Left: a batch that passes its checksum but fails to parse reports its pages `Unlogged` on top of its own log finding.

## 2026-11-02: landed, closed

`fix: report a page past the recovered log end`. The delta landed as asked: a page at or past the recovered end is `Unlogged` with or without a record, one below it with no record stays Orphan; relation-scoped VERIFY keeps the finding; a live check reports a page at or past the log's next LSN read after the page; tests reach the no-record branch, the known-damage path and the relation scope. Check exit 0, 3606 passed; deep passes over 300..396 with a floor on `Unlogged`. The round 2 live check change was not separately reviewed.

The leftover, a batch that passes its checksum but fails to parse reporting its pages twice, moves to fault-matrix.
