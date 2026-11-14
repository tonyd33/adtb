---
created-at: 2026-11-14
status: open
kind: bug
priority: 470
summary: "the evolve law search cannot hold a relation over a prelude record: its test helper expects every write state's constructor to be a user constructor"
tags:
  - tests
  - class-weak-tests
---

# The law search cannot include a relation over a prelude record

## Report

crates/adtb-lang/src/evolve_tests.rs:51-57 `named_states` looks up each write state's constructor among user constructors and panics ("a write state names objects the source holds") for a relation over `Span (Key us)`, whose constructor is the engine's. So the law search's shapes hold no relation over a prelude record, and the operators' inverses are not searched over one. With the lookup patched in a scratch copy the full law search passed. Found in the review of the prelude-ref-relation track.

Direction: resolve state names through the schema's own constructor names (engine and user); add a relation over `Span (Key us)` to the law shape.

Defect class: Tests asserting less than they claim.
