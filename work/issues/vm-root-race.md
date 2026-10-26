---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 556
summary: "Vm::put can set a bit on what has just become an inner page when the visibility map's root grows"
branch: adtb-heapg
tags:
  - engine
---

# Bug: Visibility map root race

## Report

`Vm::put` resolves its leaf without latches. If the map's root is still
a leaf and `Vm::extend` grows it before `put` takes the latch, `put`
sets a bit on what is now an inner page. Fixing it inside `put` would
latch out of the order in `docs/design/mvcc.md` §1. Expected: `put`
validates, under the leaf's latch, that the page is still the leaf for
its position (as heap pages are rechecked with `Space::names`), and
retries from the root when not. Add a threaded test with the root
growing beside writers.
