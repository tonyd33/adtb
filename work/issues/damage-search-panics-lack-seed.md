---
created-at: 2026-11-10
status: open
kind: bug
priority: 600
summary: "a panic inside the damage search's plan, run or judge names neither the seed nor the write-out, so the case cannot be rerun"
tags:
  - quality
  - test
  - sim
  - class-weak-tests
---

# Bug: damage search panics lack the seed

## Report

`search` and `pairs` (crates/adtb-sim/tests/storage/damage_search/mod.rs:751-783, 1105-1175) call `case::plan(seed, ..)` and `case::run(..)` with no seed context. Panics inside carry none: world.rs:565 (`"{o:?} is held by its own heap or index"`), the `expect("the relation's heap")` family at world.rs:528-600, stated.rs:90 (`holding` on `Holder::Log`), mod.rs:1148 (`expect("a counted write-out has a kind")`), judge.rs:594 (`&f.pages[0]` on an empty list).

Fix direction: run each plan and case under a guard that adds the seed and write-out to a panic, or pass them into the messages.

Found in the quality review of `reviewed..main` at 7bf40103.
