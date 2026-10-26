---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 717
summary: Catalog::open panics on a stored migration or roots row it cannot read
closed: 2026-10-25
tags:
  - quality
  - engine
---

# Feature: Catalog::open reports unreadable stored state

## Report

`Catalog::open` panics on stored state rather than failing: `fold` (crates/adtb/src/catalog/mod.rs:282-293) panics when a stored migration no longer parses or applies, `named` (mod.rs:240, 257) and `uint` (mod.rs:147) panic on a roots row of an unexpected shape, and `OrderKey::new(..).expect` on a bad stored key. The fold case is reachable without corruption: an engine whose checker has become stricter refuses a migration an older engine stored, and the process aborts at open with nothing the caller can report or recover from.

Fix: give `catalog::Error` a corrupt-state variant (and one for a stored migration the current engine does not accept) and return them from `open`. This differs from internal-error-path, which covers checker bugs reaching execution.

## 2026-10-25: resolved

## 2026-10-25: resolution

`open` returns `Error::Corrupt` for stored state that does not read and `Error::Unaccepted` for a stored migration this engine refuses. Remaining panics in the open path: `key_schema` and asserts in `Heap::open`. No sim test of `Unaccepted` yet: nothing can store a migration the current checker refuses.
