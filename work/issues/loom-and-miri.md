---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 320
summary: "no Loom tests of the page state word, and Miri does not run over the simulator tests"
branch: loom-miri
tags:
  - sim
  - test
---

# Feature: Loom and Miri

## Report

Milestone 3 of `docs/plan.md` asks for Loom tests of the page state
word and exits when Miri runs the simulator tests clean. Neither runs.
Miri needs a nightly toolchain in devenv.

## 2026-10-29: scope

Exit of milestone 3 as narrowed in docs/plan.md: Loom model tests of the page
state word in crates/adtb-sys/src/pages.rs (latch and unlatch, an optimistic read
against a writer, the clock against a latch), and Miri over the adtb-sys tests of
pages.rs and buf.rs, with a nightly toolchain in devenv. os/uring.rs stays out of
Miri.
