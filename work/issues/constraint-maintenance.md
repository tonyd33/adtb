---
created-at: 2026-10-25
status: open
kind: feature
priority: 740
summary: the engine neither admits constraints by the index-bounded rule nor checks them incrementally
depends-on:
  - planner
  - write-path
tags:
  - engine
---

# Feature: constraint admission and incremental checks

## Report

Implement `docs/spec/constraints.md` in the engine and
`docs/design/maintenance.md` §1, §3 and §6: the admission rule, delta
queries over base indexes checked at each statement boundary, and the
large change set path.

Prerequisites: `planner.md`, `write-path.md`.
