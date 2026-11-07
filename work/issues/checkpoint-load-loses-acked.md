---
created-at: 2026-11-07
status: open
kind: bug
priority: 420
summary: "the checkpointer crash search restarts its write values for the run below the warm load's, so a durable unacknowledged write reads as lost acknowledged data at deep seeds"
tags:
  - class-weak-tests
---

# A crash under the checkpointer's write load loses acknowledged writes at deep seeds

## Report

## 2026-11-07

Found while checking the damage search: on main (code of 6696efe4, periodic checkpoint change 2), `cargo nextest run -p adtb-sim --test storage --profile deep` fails both crash searches of crates/adtb-sim/tests/storage/checkpointer.rs at checkpointer.rs:273:

- `a_write_load_keeps_what_recovery_reads_and_the_dirty_page_table_bounded`: "crash after event 360: Pid(5) holds 4, acknowledged 96".
- `under_atomic_page_writes_a_write_load_keeps_recovery_and_the_dirty_page_table_bounded`: "crash after event 2873: Pid(7) holds 1, acknowledged 799".

Both pass at the default profile. A crash image reopens clean with a page holding less than was acknowledged: acknowledged data lost. Not yet known whether it predates the in-session recycle (6696efe4): bisect against 51e37d1e first.

Defect class: Multi-step change not atomic under failure, crash or race.

## 2026-11-07, cause

Not data loss. The crash search calls `load` twice (a warm load, then the run) and `load` numbers values from 1 each call, so the run's values restart below the warm load's. A durable but unacknowledged first write of the run (allowed to survive) is smaller than the last acknowledged warm value and fails `acked <= v`. Fails identically at 51e37d1e. A test-only fix (values numbered past the warm load's, two computed crash points as regression) is in slot-1, change qxtxotqw, paused by the structural freeze.

Defect class: Tests asserting less than they claim.
