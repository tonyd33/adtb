---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "a crash between a history heap insert and its key index insert leaves an unindexed copy that the next vacuum copies again, and the crash test cannot see it"
tags:
  - quality
  - engine
---

# A crash mid-move leaves an unindexed history copy that the next vacuum copies again

## Report

`history::keep` (`crates/adtb/src/history.rs:94-111`) calls `past.insert_version` and `into.index.insert` as two pager updates, and `held` (`history.rs:114-133`) looks for an earlier copy only through the history key index. After a crash between the two calls the next `vacuum_keeping` inserts the version again, contradicting "a later vacuum copies none twice" in `docs/design/mvcc.md` section 10.3. Reads stay correct because `history::rows` drops duplicates by `(xmin, xmax, row)`, and `expire` frees both copies eventually, but the history heap holds an orphan until then.

The crash test's `duplicated` check (`crates/adtb-sim/tests/history.rs:393-411`) walks only the history index's entries, so it cannot see an unindexed version.

Fix: make the heap insert and the index insert one pager update, or have `held` fall back to the page. In the test, also count history heap versions no index entry names.
