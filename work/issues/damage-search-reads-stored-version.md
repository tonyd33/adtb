---
created-at: 2026-11-09
status: open
kind: bug
priority: 200
summary: "the damage search infers which migrations a file holds from the schema an open returns, so an open that fails after its migration commits (a verify-only engine) hides the commit and the search charges a later open"
tags:
  - testing
  - class-weak-tests
---

# Bug: the damage search infers the stored migrations from the open's schema

## Observed

When an open fails after its migration committed, `Engine::open` returns the verify-only engine (`Engine::unread`), whose schema has no app relations. The damage search (crates/adtb-sim/tests/storage/damage_search) records the migration as not applied, and charges the next open that loads it. The caller has no way to learn which migrations the file holds from that engine.

## Expected

The search, and a caller given a verify-only engine, read the migration version the file stores, so a commit followed by a failed open is attributed to the open that committed it.

## Reproduction

The `APPLIED_UNDER_MARK` rows of migrate-applies-under-mark. Found in structure-damage-policy step 5.
