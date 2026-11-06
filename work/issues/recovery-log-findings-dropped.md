---
created-at: 2026-11-05
status: open
kind: bug
priority: 345
summary: "recovery's log findings (a stale block past the end) are kept in Recovery.log, discarded by the caller, then erased by clear_tail"
tags:
  - class-damage-unreported
---

# Recovery's log-tail findings are never reported

## Report

## 2026-11-05

From the damage-reporting audit, confirmed in code: storage/recovery.rs builds `Recovery { log: s.findings, .. }` after `clear_tail` has zeroed or removed the blocks, and api/mod.rs and create.rs discard the `Recovery` (`Ok((p, _))`). A valid block with a sequence at or below the last accepted, past the end of the log, is `LogProblem::Stale` (block.rs:417-421); storage.md §7's table says it is reported as log damage. Nothing reports it, and the evidence is erased before any verify. Direction: record the findings durably (a mark or a verify-visible log finding) before `clear_tail`. Defect class: Damage reported as nothing.
