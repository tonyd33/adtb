---
created-at: 2026-10-27
status: open
kind: bug
priority: 792
summary: "a keyed read of the past misses a row while vacuum moves its version to history, and after a crash in that gap, because vacuum deletes index entries before copying"
tags:
  - quality
  - engine
---

# A keyed read of the past misses a row while vacuum moves its version to history

## Report

Latent until `history-catalog` wires the history layer in; only tests call it today.

`clean` in `crates/adtb/src/vacuum/run.rs:476-517` first retains every index of the sum against every dead tid (`run.rs:490-496`), and only then runs `history::keep` and prunes per page (`run.rs:499-505`). Between the two, a version a held marker sees and the window keeps is still in the current heap, has no entry left in the current key index, and has no copy yet in the history heap or its index. `history::probe` (`crates/adtb/src/history.rs:183-224`) reaches current versions only through `index.probe_ctor` and then the history index, so for that key it returns `None` or a newer row. That happens to a read beside a running vacuum, and after a crash in the gap until the next vacuum. It breaks the postcondition in `docs/design/mvcc.md` section 10.3 that a read at a marker returns each row the marker's snapshot saw, once, however vacuum moves versions.

The tests pass because `reads_of_the_past_beside_a_vacuum_moving_their_versions_read_every_row_once` (`crates/adtb-sim/tests/history.rs:962`) and `a_crash_at_every_flush_while_vacuum_moves_versions_loses_no_past_row` (`history.rs:757`) read only through `read_at` (the full scan in `history::rows`); `probe_at` is called only in the quiet test at `history.rs:475`.

Fix: in `clean`, run `history::keep` for every page of `dead` before the index retain. Call `probe_at` beside `read_at` in the concurrent test and in the crash loop, before and after the second vacuum.
