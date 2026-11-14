---
created-at: 2026-11-13
status: open
kind: bug
priority: 500
summary: "a pending constraint or check drop is keyed by module and name, so adding a new constraint of that name ends the pending drop that a pinned bundle still holds"
tags:
  - engine
  - quality
  - class-identity-by-spelling
---

# Adding a constraint under a pending drop's name ends the drop

## Report

A dropped constraint or check that a pinned bundle still holds stays enforced as a pending drop. `Pending` (crates/adtb/src/catalog/readers.rs:131) holds it by `module` and `name`, and the held roots rows store no id, although `Dropped` (readers.rs:166) has the `CatConstraintId`. When a later migration applies, the code at readers.rs:477 splits off as "redeclared" every pending constraint or check whose module and name the new schema declares (`declares`, readers.rs:881). That removes it from `pending` and from the enforced source.

Since step 6a-2, a constraint added again under a dropped one's name is a new object with a new id and possibly another body. The bundle pinning the old version then loses the enforcement of the constraint its version declared, and rows that violate it can be written while it is held.

Repro sketch: bundle B pinned at v1, which has `constraint c = from u in users where u.name == "x" select u`. v2 runs `drop constraint c`, which is pending and held by B. v3 runs `add constraint c = from u in users where u.name == "y" select u`. Expected: `pending_drops` still lists `c` held by B, and inserting a user named `x` is refused. By reading: `c` leaves `pending_drops` and the insert applies. No test adds a constraint under a pending drop's name.

Found by reading; not reproduced.

Ruling (lead, 2026-11-13): a constraint added under a pending drop's name does not end that drop. The re-added constraint is a new object with its own id; the old one stays enforced as a pending drop until the last bundle holding it releases it, so both are enforced at once, told apart by id, and a violation names the one it breaks.

Fix direction: key pending drops of constraints and checks by `CatConstraintId` in memory and in the held rows, and decide `declares` by id.

Defect class: class-identity-by-spelling
