---
created-at: 2026-10-26
status: open
kind: feature
priority: 784
summary: "the engine refuses ad hoc commands in process, nothing plans or runs Adhoc.commands, and reindex picks the first of two identical declared indexes"
tags:
  - api
  - lang
---

# Feature: Run ad hoc commands in the engine

## Report

Left by the front end commands track:
- `adtb/src/api/text.rs` still refuses every command in process, and the engine has no bundle registry to pass to `check_adhoc_recording`, so `explain B::q(args)` cannot be checked there.
- No consumer plans or runs the checked terms in `Adhoc.commands`.
- When two declared indexes of a relation have the same fields and scope (a `unique (f)` and an `index (f)`), `reindex r index (f) in S` picks the first.
