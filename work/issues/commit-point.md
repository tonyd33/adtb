---
created-at: 2026-11-17
status: open
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
