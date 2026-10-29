---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 822
summary: "log grants are not rows of catalog::grants, a migration may grant to a missing role, and replay has no dormant grants"
branch: log-grant-rows
depends-on:
  - grant-carry
tags:
  - engine
---

# Feature: log grant rows

## Report

Slice 2a of roles-and-grants (provider order step 1, catalog part). `Source::grants` (landed by grant-carry) is not projected into `catalog::grants`, whose `kind: Log` rows catalog.md section 2 defines; the only rows today are Deployment capability grants written by api/server.

- Each log grant held after a migration is a `Log` row of `catalog::grants`, in the migration's transaction. A row's id comes from the grants relation's durable id counter, shared with Deployment rows; a grant that still stands after a carry, rename or option change keeps its row and id; only the affected Log rows are inserted, updated or deleted. Ids are never derived from source order, catalog ids or a numeric partition. Refused migrations may consume ids, never publish rows; reopen never reuses an id.
- `catalog::deploy` assumes no migration writes a relation it writes (deploy.rs, `deploy` preconditions; schema-change.md section 2). Replace that with an explicit row-class contract for `grants` in docs/design/schema-change.md: migrations own Log rows, deployment owns Deployment rows, both in one relation, each transaction atomic.
- Native migration refuses a grant operator naming a role that does not exist in its applying transaction, publishing nothing. Replay and recovery (`Source::migrate` folding the log) never refuse a committed prefix for a missing role: such a grant becomes a dormant Log row, reported, allowing nothing, never silently activated when the role is created later. Dormancy is deployment metadata outside the schema hash.
- Existing permissive tests such as adtb-sim/tests/oplog.rs:1570-1588 (grant to a missing `support` role) change meaning; update them to create the role or assert the refusal, not reinterpret them as replay.

Out of scope: live bundle rechecks on migration and revoke (slice 2b), registration, call grants.

## Fix

Audit, local: work/briefs/report-authority-next-audit.md, "Sequenced remainder" item 1 (lines 143 to 191) and "Acceptance" canonical integration (lines 282 to 290).

## 2026-10-29: lead ruling on dormant activation

Implemented in log-grant-rows (de970900, 33cfc45e, 50a52062); contract in docs/design/schema-change.md 1.1 "Grant rows". Ruling: creating a role never activates a dormant Log grant naming it. A grant operator in a later migration that names the now existing role and the same privilege clears the dormant mark on that row, keeping its id; it is an explicit act of a role holding `schema`, not a silent activation. spec-revision: catalog.md section 2 should state this and the `target` text form of a carried grant (current names, `frozen` field lists, `except` retag pairs), which "as written" no longer describes.
