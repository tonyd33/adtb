---
created-at: 2026-10-25
status: open
kind: feature
priority: 320
summary: "no Loom tests of the page state word, and Miri does not run over the simulator tests"
tags: [sim, test]
---

# Feature: Loom and Miri

## Report

Milestone 3 of `docs/plan.md` asks for Loom tests of the page state
word and exits when Miri runs the simulator tests clean. Neither runs.
Miri needs a nightly toolchain in devenv.
