---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 570
summary: "verify rereads the whole log on every call, checks only commit log page kinds, and skips cross-heap moved chains"
branch: adtb-vac2
depends-on:
  - verify-structural
tags:
  - engine
---

# Feature: Deeper and cheaper verify

## Report

From the structural verify:
- `verify` reads the whole retained log through `Pager::verify` on
  every call, which made a model test five times slower when it ran
  after each step. Let a caller skip the log, or verify it
  incrementally from a remembered position.
- The commit log check confirms only page kinds and ownership. Add a
  `CommitLog` check that the status pages cover `[bound, reserved)`.
- Cross-heap `moved` chains are not validated, because per-heap
  watermarks let them dangle. State in `docs/design/mvcc.md` which
  dangling chains are allowed, and check the rest.
