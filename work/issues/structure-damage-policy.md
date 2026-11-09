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
