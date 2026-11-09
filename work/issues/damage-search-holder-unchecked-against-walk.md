---
created-at: 2026-11-09
status: open
kind: bug
priority: 250
summary: "the damage search classifies a page by the owner in its write-out header, so it cannot see where the engine's holders walk classifies the page differently (unreached, free, retired)"
tags:
  - testing
  - class-weak-tests
---

# Bug: the damage search's holder is not checked against the engine's walk

## Observed

The damage search (crates/adtb-sim/tests/storage/damage_search) takes a damaged page's holder from the owner in its write-out header. The engine maps a page by walking its structures. Where the two disagree, the search expects the mark of the header's holder and never asks the rows for an unreached page: `Held::Unreached` is never produced, so `(_, Unreached, Opening)` is listed `Never` in `unasked()`.

## Expected

The search derives a page's holder from an independent walk of the file as written, and asks the unreached, free and retired rows when that walk and the header disagree.

## Reproduction

Read `unasked()` after the damage profile. Found in structure-damage-policy step 2, round 3.
