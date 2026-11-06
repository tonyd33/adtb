---
created-at: 2026-11-05
status: open
kind: bug
priority: 330
summary: the verify that first finds commit-log damage quarantines the database and returns Fatal instead of its findings
---

# First verify on a stale commit-log page returns Fatal, not its findings

## Report

## 2026-11-05

Found by the clog-lost review, reproduced on main. When a commit-log status page is stale (a lost write), the first `db.verify(None)` returns `Err(Fatal { reason: Quarantined { what: "the database" } })`. Its path is `api::Database::verify -> query_text -> Engine::run -> resumable -> write::Database::quarantining -> mark -> set_mark -> Pager::quarantine(Database)`. Only a second verify returns the `StalePage` row and the `Structure` rows. The call that found the damage reports an error, not the damage. storage.md §8 says a verify always runs on a file under the mark and answers at least one `Structure` row naming the mark.

The test crates/adtb-sim/tests/sim/clog_lost.rs `a_commit_whose_status_page_lost_the_write_reads_committed_until_verify_marks_the_database` pins the current two-call sequence and should change with the fix.

Direction: a verify that sets a whole-database mark finishes and returns its findings, with the mark taken after.

Defect class: Damage reported as nothing.
