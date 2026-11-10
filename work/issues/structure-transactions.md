---
created-at: 2026-11-07
status: open
kind: feature
priority: 15
summary: "write::Database is the whole engine under one name, Txn has no Drop so begin is paired by hand at 10 sites, lock preconditions are comments, and errors cross four layers by hand and as text"
---

# Feature: Transactions and errors with owners in the write path

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-exec.md (local). api/verify.rs:459-462 returns through `?` after `begin` without aborting. 16 error enums, 20 `From` impls, 40 `{e:?}` sites; `ends_transaction` a hand-kept list. Executor inputs are pub-field structs filled per site; SSI read recording chosen per site; execution.md §1.3's 9 plan invariants checked nowhere, 30 `Broken` checks at run time instead.

Exit: a transaction guard that aborts on drop; lock preconditions as guard parameters; errors typed through to the api with no `{e:?}` conversion; executor inputs built by one constructor each; a plan validator for execution.md §1.3.

## 2026-11-08, from the version-owner design

The scoped snapshot: a registered reader's snapshot holds back only its plan's heaps, and nothing checks that it reads only those (`Scope::Heaps`); a wrong scope lets vacuum free a version the reader then walks. Lead ruling (structure-version-owner Q7): the `Reader` carries the `Scope` it was taken with and `Store` checks each heap against it, in this program, after structure-version-owner step 2 adds the `Reader`.

## 2026-11-10, design and rulings

Design report: work/briefs/design-transactions-r1-report.md (local). Program of 13 steps; order 1 and 2 first, 3 after 1, 4 after 3, 5 after 1 and 3, 6 after 5, 7 independent, 8 then 10, 9 and 11 after 10, 12 and 13 independent. Steps 4, 8, 9, 11 and 13 change client-visible behaviour; the rest do not.

Lead rulings:

- Q1, the transaction guard: an owned guard. The transaction resources (`txns`, `ser`, `readers`, `waits`, the dead counts) move into a `write::Session` behind one `Arc` that the `Txn` holds, so a server `Call` can keep its `Txn` across worker steps. `abort` splits into an infallible in-memory release (hold, sid, readers, waits, dead counts, the xid out of `running`) and the fallible commit-log record. `commit(self)` and `abort(self)` consume and mark the guard ended first; drop releases and records the abort best effort, counting a failure. An `Err` in `commit` aborts and returns the commit's error. A failed abort record leaves the commit log saying in progress, which readers treat as not committed and recovery settles as aborted; mvcc.md §4.1 and §9 state it in step 3. Reason: a borrowing guard cannot live in a resumable call, a whole-engine `Arc` hides what the abort depends on, and an orphan list leaves the watermark pinned until a later call.
- Q2, panics caught as findings: reindex catches only the failures its job returns as `Err`; a panic ends the process, as fatal-split rules, after step 3. Scrub keeps catching, every resource inside the catch a guard, until typed shape faults cover what it reads; ticketed separately. Reason: a reindex panic is an engine invariant and the old index stays visible; scrub's purpose is to read arbitrarily bad pages.
- Q3, lock preconditions: positive preconditions become witness parameters (`admit_commit(&Ordered)`, `rebuild(&VacuumHold)`, a `BundleWriter` guard replacing `bundle_writer: Lock<()>`); negative ones are entry assertions under `latch-checks` from the pager's per-thread latch count and a ser-holder id; `write::Database`'s four locks get ranks checked as `heap::Stage` checks latches. Positive first.
- Q4, the error spine: a `Class` on `write::Error` decided by variant (operating, damage, refused by a mark, checker guarantee, engine invariant); `ends_transaction`, `fail`'s mapping and `of_mapped` read it. `write::Error::Exec(exec::Error)` replaces the copied arms; `Transactions(mvcc::Error)` typed; one `From<catalog::Error>` lifts damage to `Page`; engine-state `Broken` variants panic naming their invariant. Not one shared enum: it would couple the executor to write-path variants.
- Q5, executor inputs: one private-field constructor per kind of read (serializable, safe, past, migrating), each building its heaps from `program.reads` and failing with `Internal` naming a missing binding instead of panicking.
- Q6, the plan validator: always on, at the end of `plan::lower`, refusing with `plan::Broken`, for P1, P3, P5, P6, P8, P9 and the `Fix` and `Top` shapes. Step 7 measures its cost against planning; past 5% of planning time on the planner tests it reports before landing. Reason: one linear walk, and it replaces release-build panics on a planner bug.
- Q7, scope: `Transactions::take(scope)` returns a `Hold` guard; the `Reader` carries its scope and keeps its hold alive; `Store` refuses a heap outside the scope with `Internal`. Scoped publishing for registered calls (mvcc.md §9) is a later feature.

## 2026-11-10, step 1 landed

Landed ca206853, full check exit 0 (3617 tests). `Transactions::take(scope)` returns an owned `Hold` guard with its snapshot; the guard reaches the transactions state through a `Weak<dyn Release>`, so it does not keep the engine alive and needs no generic parameter, and drop releases the hold once on every path. `Transactions::release` is gone, `Holds::release` is private to mvcc and `SnapshotId` is not exported: releases by id outside mvcc 16 to 0. The snapshot's commit position moved into the guard as `Taken`, so a read-only `Txn` dropped without abort still logs its safe marker. Commit releases its hold just after the ser lock, before `wait_durable` (holding longer is conservative); abort under it; mvcc.md §9 states both and that a read-write transaction's hold is released only after `log_safe` runs for it. No `?` between `take` and release was reachable; the enforcer is `mvcc::clog_tests::a_dropped_hold_releases_its_heaps`, failing on main. Protection narrowed: a `write::Txn` dropped without ending now releases its hold, while its xid, sid, readers entry and waits leak until step 3.

## 2026-11-10, step 2 landed

Landed 6101a53c, full check exit 0 (3624 tests). `Transactions::begin` returns a `Begun` guard (through `Weak<dyn Release>`, as `Hold`); `commit` and `abort` take it by value and mark it ended first; a drop aborts best effort. Abort is split: the id always leaves `running`, and the commit-log record is attempted, a failure counted (`unrecorded_aborts`, testing only) and not returned; a thread that is panicking records nothing. A failed commit returns its own error. Ids that left `running` without a record stay in `State::unresolved`, which bounds each reservation's `resolved` and vacuum's `clean_below` (`Transactions::resolved_below`); mvcc.md §4.1, §4.3, §9 and §10 and the `Snapshot` invariant say resolved or unresolved, an unresolved id treated as not committed. The pager has a closed state set first in `Pager::close`; `Transactions::admit` refuses after it (`mvcc::Error::Closed`), so a guard dropped after close releases in memory and writes nothing (embedding tests at server/embedding.rs reach that path).

Reproduced and fixed (class-not-atomic): a migration or an index release failing on an unreadable catalog root, which marks nothing, left its id running and the watermark pinned until restart (write_states_tests, both failing on main). `txns.begin(` returning a bare `Xid` outside mvcc 6 to 0; `?` windows between begin and end 4 to 0. A dropped `write::Txn` now aborts its xid; its sid, readers entry and waits still leak until step 3.
