---
created-at: 2026-11-07
status: open
kind: feature
priority: 14
summary: "damage policy is split over four layers: two holders functions state opposite rules for the roots heap, 49 sites set or read marks, open bypasses the commit-ordered mark"
---

# Feature: One owner for damage mapping and marks across storage, write, catalog and api

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-exec.md (local). write/quarantine.rs:48 says roots heap damage marks nothing; catalog/mod.rs:850 says it marks the database. 3 open-time sites call `pager.quarantine` directly. The quarantine rewrite is applied by hand at 3 sites, and api/statements.rs:113-114 must read `ends_transaction` before it. The gated damage findings walk-damage-marks-database, unread-walk-marks-database, migrate-under-mark-silent and migrate-applies-under-mark are re-planned here.

Exit: one mapping function and one mark setter, the policy in storage.md §8 only, the rewrite applied in one place.

## 2026-11-09, design and rulings

Design pass round 1 (work/briefs/design-damage-policy-r1-report.md, local): detection has one storage entry, but about 20 sites in 8 files decide what a fault marks, two `holders` and `opens_from` disagree by phase, `Pager::quarantine` has 8 production callers (two in session without the commit ordering), `Transactions::quarantine` is a second record of a database refused whole, the quarantine rewrite is called by hand at 3 sites, and the damage search's expected marks come from the code under test. Owners: `policy::mark(Fault, Holder, Phase)` transcribing storage.md §8; one inventory `catalog::Structures`; one mark setter taking an `Ordered` or `Opening` witness; `Pager::whole`; `fail -> Mapped` with `Quarantined { by }`; a `Damage::Shape` variant. Eleven steps: 1 to 3, 4a and 6 keep behaviour; 4b, 5 and 7 to 11 change it, each pinned.

Lead rulings: (Q1) kept rows get a heap of their own in structure-catalog-roots, whose damage marks nothing, and the roots heap then marks the database in every phase; until then `mark` keeps today's two answers as one row keyed by `Phase`, stated in §8. (Q2) storage.md §8 is amended to the walk-damage ruling in step 9: a commit's met page marks its header's owner when the header reads, the database otherwise, and the record stays unmapped for the call-end walk; the Recycled rows decide whether a stale header's owner is acceptable, and a wrong-relation mark found there blocks the step. (Q3) the unreached page marks the database once running and nothing at open, one row. (Q4) `Damage::Shape` in the record and the list; clients keep seeing `Checksum` until a spec change adds a row. (Q5) an open under a whole mark is refused only when the log has migrations past the file's, as `MigrationRefused` with a `Fatal { Quarantined }` reason, through spec-revision in step 10; an open with the file's own log still verifies and dumps. (Q6) mapping at the end of a call that returns `Ok` belongs to structure-transactions. (Q7) step 6 lands before structure-transactions' error retyping; that program extends `Mapped`.

## 2026-11-09, step 1 landed

What a detected fault marks is decided by one table, storage/policy.rs: `mark(Fault, Holder, Phase) -> Mark` (`Mark::Held` for a page held for the open only), `marks` for several holders (the database standing alone), `checksum_mark` for a walk stopped by damage. `Holders::walk` and `holders_meeting` return holder sets; `map_damage` and `map_records` apply `mark`; storage's own rows (bitmap, full list, met, marks page, list page, map page, unarmed recovery gate) go through it; `refuse_list` takes its phase from its caller. `LARGE_MARK`, `UNHELD_MARK` and `relation_mark` are deleted. No behaviour change. storage.md §8 states the roots heap and the unreached page as one row each keyed by phase, and a retired page (a page of a tree a swap replaced) marking nothing. policy_tests.rs checks every `(Fault, Holder, Phase)` the walk reaches against 178 literal rows written from §8, §6 and §7, each visited once; `DIFFERS_TODAY` lists the 20 rows where an unreadable page marks something today though §8 says nothing, shrink-only, for step 8. New structure-check row `database_marks` (`Quarantine::Database`, `Mark::Database`, `Mark::Held` outside policy.rs and the codec) 22 to 0. Inputs for step 4a: setter closures, direct `pager.quarantine` and `hold_quarantine` callers, finding and `OrphanEntry` index marks outside the table, api's `LogUnproven` guard choosing which faults reach the table, `marks` dropping a `Held` mark's hold, recovery ignoring the holder for `Unarmed`, lsnmap passing `Phase::Running` during the open. Input for step 8: `holders_meeting` special-cases only `Bitmap`, so a record on the marks page, the list page or an LSN map page maps as `Unreached`. Next: step 2.
