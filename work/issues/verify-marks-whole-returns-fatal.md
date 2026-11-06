---
created-at: 2026-11-05
status: resolved
closed: 2026-11-05
kind: bug
priority: 330
summary: the verify that first finds commit-log damage quarantines the database and returns Fatal instead of its findings
tags:
  - class-damage-unreported
---

# First verify on a stale commit-log page returns Fatal, not its findings

## Report

## 2026-11-05

Found by the clog-lost review, reproduced on main. When a commit-log status page is stale (a lost write), the first `db.verify(None)` returns `Err(Fatal { reason: Quarantined { what: "the database" } })`. Its path is `api::Database::verify -> query_text -> Engine::run -> resumable -> write::Database::quarantining -> mark -> set_mark -> Pager::quarantine(Database)`. Only a second verify returns the `StalePage` row and the `Structure` rows. The call that found the damage reports an error, not the damage. storage.md §8 says a verify always runs on a file under the mark and answers at least one `Structure` row naming the mark.

The test crates/adtb-sim/tests/sim/clog_lost.rs `a_commit_whose_status_page_lost_the_write_reads_committed_until_verify_marks_the_database` pins the current two-call sequence and should change with the fix.

Direction: a verify that sets a whole-database mark finishes and returns its findings, with the mark taken after.

Defect class: Damage reported as nothing.

## 2026-11-05 resolved

Landed as `fix: return a verify's findings when they quarantine the whole database`. Cause: the verify's own snapshot checks read statuses through the normal path; `Transactions::status` met the stale status page and returned an error that left `verify_run` before any row was built, and `quarantining` turned it into Fatal. Now, when the snapshot checks fail on the commit log's own damage (never `Unreadable`, which propagates), verify answers what a file marked at open answers plus that page's finding whatever the scope, keeps the findings it had gathered (an index Orphan found before the status page is still answered and marked), and marks the database after the rows, through `quarantine_findings`, outside `verify_checks`. The search test reverts each page an insert changes and requires the first verify to answer that page's exact StalePage and the expected mark.

Review found `verify r` dropping the commit-log damage with no mark, an error guard catching every error, and gathered findings discarded; all fixed. 7 mutants killed. `check --changed` 3413 passed, campaign 48.

This is a site fix in the class damage-marked-at-detection now owns; the fallback goes away when marking moves to detection. Left: clog-root-lost-write-panics (the search skips the root), sim-first-verify-task-deadlock (the server test runs a query first).
