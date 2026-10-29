---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 299
summary: the reference evaluator recurses per match arm and overflows a 2 MB thread on a scalar match with many arms
closed: 2026-10-29
tags:
  - reference
---

# Bug: Reference evaluator overflows the stack on wide matches

## Report

Fuzz seed 55 of the soundness test, a scalar `match` with many arms, overflows a 2 MB test thread inside the reference. The soundness fuzz test runs on a 256 MB thread to pass. The evaluator should not need stack proportional to arm count, and its depth should be bounded the way the front end bounds nesting.

## 2026-10-28: dispatched

A fresh writer owns this in workspace reference-recursion on main 1eb25c53, in adtb-reference and the soundness test stack size only: match arms and any other per-element recursion become loops, depth stays bounded by nesting, and FUZZ_STACK drops to the default if seed 55 passes. Gates: suite over adtb-reference and adtb-testkit, clippy, fmt, house-check, size-check. Local brief: work/briefs/brief-reference-recursion.md.

## 2026-10-28: writer froze the source

Frozen 7ce0bded2f39fb94cb1fb605e2d5eb04b3d84755 on main 1eb25c53 (workspace reference-recursion). `Run::eval` follows the taken branch of case, if and let in a loop, with the rest in `Run::step`; `Run::block` and `Run::write` loop the same way; `impl Drop for Frame` unlinks environment frames in a loop. Stack now grows only with term nesting, bounded by the parser's `MAX_DEPTH` (64). New crates/adtb-reference/tests/wide.rs, five tests on 2 MiB threads; `FUZZ_STACK` is now 2 MiB. Seed 55 overflowed only at opt-level 0, where it now passes. The checker's per-arm recursion is ticket checker-match-recursion. Writer gates: suite over adtb-reference and adtb-testkit (2523 passed, 7 skipped), clippy, fmt, house-check, size-check. Running: the lead's branch check and a fresh review.

## 2026-10-29: landed

Landed as 93da74f8 after a clear review; the main check passed on it. `Run::eval`, `Run::block` and `Run::write` loop over tail positions, so the reference's stack grows with nesting depth, not with the length of a `let` chain, guard chain or block. crates/adtb-reference/tests/wide.rs runs the wide cases on 2 MiB threads.

## 2026-10-29: resolved
