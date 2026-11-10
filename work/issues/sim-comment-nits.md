---
created-at: 2026-11-10
status: open
kind: bug
priority: 900
summary: "Sim's Drop doc restates its body and omits the cycle it breaks, and policy_tests cites storage.md in a comment"
tags:
  - quality
  - test
  - class-none
---

# Bug: comment rule breaks in the sim drop and policy tests

## Report

- crates/adtb-sim/src/sim.rs:168: `Drop for Sim`'s doc "Drop the plan's rules and actions." restates the body. The invariant a reader needs is missing: planned `Action::Run` closures hold `SimRuntime` clones, and the drop breaks that cycle.
- crates/adtb/src/storage/policy_tests.rs:99: "The marks storage.md states, ..." cites a file in a comment.

Found in the quality review of `reviewed..main` at 7bf40103.
