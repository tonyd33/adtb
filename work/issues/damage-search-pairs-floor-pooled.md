---
created-at: 2026-11-10
status: open
kind: bug
priority: 300
summary: "the fault-pairs search floors records and marks over all second faults together, and counts an unreadable-page second as reached when the open succeeded, not when its read failure fired"
tags:
  - quality
  - test
  - sim
  - class-weak-tests
---

# Bug: the pairs floors are not per second fault

## Report

- `assert_reached("pairs whose file held a damage record", recorded, 1)` and the same for `marked` (crates/adtb-sim/tests/storage/damage_search/mod.rs:1192-1193) count over the whole `pairs()` call. In `a_lost_write_beside_a_crash_in_a_fold_keeps_every_record_and_mark` one fold point holding a record satisfies the floor for every `Fold` second, so `record_lost` and `mark_lost` can judge nothing for one of them and pass.
- `reached` (mod.rs:1214-1218) counts `r.opened` for `Second::UnreadableOnce` and `Second::UnreadableLasting`; only the log, list and marks seconds count `r.fired`. A pair whose planned page read failure never struck counts toward its floor. damage-search-fault-pairs's resolution states that each second's floor counts cases whose planned fault fired, reads included.

Fix direction: keep `recorded` and `marked` per `Second` and floor each; count `fired` for both page-unreadable seconds.

Found in the quality review of `reviewed..main` at 7bf40103.
