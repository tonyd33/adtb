---
created-at: 2026-11-17
status: resolved
closed: 2026-11-17
kind: feature
priority: 119
summary: "a durable commit is followed by a fallible step whose Err reaches the caller, so a caller is told a change failed that is durable"
tags:
  - engine
  - class-not-atomic
---

# Feature: Commit point

## Request

## Proposal

## Open questions

## Request

Mechanism for class-not-atomic group A, from the class count of 2026-11-17. A durable commit is followed by a fallible step (`free_leaked`, `end_freed`, `settle`, `release`) whose `?` returns Err to the caller: crates/adtb/src/catalog/migration.rs (commit, then `free_leaked`), catalog/pending.rs (pending-drop commit, then `free_leaked`, `end_freed`), heap/write.rs (page update, then `settle`, each `release` skipping the rest), heap/dir.rs (two allocation sites, then `settle`), and possibly catalog/mod.rs (create commits, then `update_root`).

## Proposal

`commit` returns a `Committed` token; a step after it takes the token and returns nothing a caller can propagate: its failure is recorded as damage for the next mapping or open, which already tolerates it, and the marker stays for the next open. Releases run before settle. Enforcer: the type, plus a fault sweep that reaches each post-commit step and asserts the caller sees success and the next open is consistent.

Exit: migrate-applies-under-mark and heap-write-settle-skips-release resolved; the damage search's APPLIED_UNDER_MARK known rows removed.

## Resolution 2026-11-17

Landed 2026-11-17 as `fix: report success for a durable change whose post-commit step fails`. `Transactions::commit` returns `mvcc::DurableCommit`, minted only there; the post-commit steps of migration.rs and pending.rs take it through `Catalog::end_committed` and leave the change marker for the next open on failure; catalog.md §11.2 states that a call finding damage after its change committed succeeds. Heap writes were not commit points: they keep failing the statement, ordered so no release is skipped. Sweep keyed by site. Damage search: the 6 APPLIED_UNDER_MARK rows removed; 3 rows name migrate-under-relation-mark. Resolved heap-write-settle-skips-release. migrate-applies-under-mark stays open for the open's later steps (api/mod.rs) after a committed migration.
