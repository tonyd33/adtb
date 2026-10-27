---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 784
summary: "the engine refuses ad hoc commands in process, nothing plans or runs Adhoc.commands, and reindex picks the first of two identical declared indexes"
closed: 2026-10-26
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

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in rqxztvot. The engine keeps a bundle registry (empty until bundles exist) and runs `explain`, `explain analyze` (through a trial run that is undone), `vacuum` and `set` in process at their place among the statements. `reindex` names every matching declared index but does not rebuild; its two tests are ignored until it does. Other commands are refused. The rest is `command-follow-ups`.
