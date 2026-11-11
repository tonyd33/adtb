---
created-at: 2026-11-11
status: open
kind: bug
priority: 480
summary: "no test reaches an explain analyze trial whose take back fails, the path that ends the transaction after every statement succeeded"
tags:
  - quality
  - engine
  - test
  - class-weak-tests
---

# No test reaches a trial whose take back fails

## Report

An `explain analyze` trial runs its statements and takes them back (crates/adtb/src/write/mod.rs:1006-1035, `try_items`). When every statement succeeded and the take back fails, the transaction ends with the undo's error, but no test reaches this path; the take-back-failure tests cover only `statements`. `analyzed` (crates/adtb/src/api/command.rs:612) drops the ending flag and relies on the undo error's class to end the transaction, consistent today and unchecked.

Direction: an api test of `explain analyze` on an insert with the heap refused between the run and the take back, asserting the exact reply, that the transaction ended and the mark set.

Defect class: Tests asserting less than they claim.
