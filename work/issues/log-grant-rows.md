---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 822
summary: "log grants are not rows of catalog::grants, a migration may grant to a missing role, and replay has no dormant grants"
closed: 2026-10-29
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

## 2026-10-29: review blocks ed39a084

Fresh review (work/briefs/review-log-grant-rows.md, local): ids, row ownership, open matching and target round trip hold. Three blockers, one cause: dormancy lives only in the row. Lead rulings:

- B1: a grant operator that merges into an existing (dormant) entry skips the missing-role check. Every grant operator of a native migration names an existing role or the migration is refused, merges included. A revoke may name a missing role, so dormant rows can be removed.
- B2: a prune merge keeps the first entry's row, deleting an active row in favour of a dormant one. A merged entry is active if any merged entry is, and keeps an active entry's row id.
- B3: reverting a revoke puts the grant back active. Dormancy is part of `Source` grant state (outside the schema hash), so every inverse restores it exactly; reverting the migration whose regrant cleared dormancy makes the row dormant again.
- An unlisted regrant whose frozen field set differs is a different privilege: a new active row, the dormant one unchanged.
- Verify with a witness: a first open that both creates the counters page and runs a migration making a populated relation `generate sequence`; a colliding first generated insert blocks.
- Non-blocking: dormant rows found at open are not reported (`api/mod.rs` drops `record_grants`'s result); `Catalog::replay` has no engine caller; `Source::restated` beside `unstated` (grant-carry-follow-ups); docs line that a fresh database cannot open a log granting to any role but the owner.

## 2026-10-29: second review blocks 28fc0337

B1 to B3 hold in principle; the constructor-10 roots row is atomic with the grant rows and outside the hash (work/briefs/review-log-grant-rows2.md, local). Branch check on 28fc0337 passed (root 2789, campaign 47).

- R1: a prune merge ORs a dormant entry's grant option into the active row it keeps. Ruling: a grant option held only while dormant never becomes active. A prune merge of active and dormant entries takes the option from the active entries only; an activating regrant gives the row the regrant's own option, not the dormant entry's.

## 2026-10-29: landed

R1 fixed as ruled (lead review of the delta). Landed as 5b219244..8f6035ae. Branch check on 28fc0337 passed (root 2789, campaign 47); main check on 8f6035ae passed (root 2799 passed, 7 skipped; campaign 47). Follow-ups in log-grant-rows-follow-ups. Next roles-and-grants slice 2b: live bundle rechecks on migration and revoke; relation-grant checks skip dormant rows.

## 2026-10-29: resolved
