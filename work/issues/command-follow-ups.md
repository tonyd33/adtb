---
created-at: 2026-10-26
status: open
kind: feature
priority: 789
summary: "reindex does not rebuild, settings are neither seeded nor read, explain analyze has actuals only at the root, and verify, compact, print schema and the other commands are refused"
tags:
  - engine
  - api
---

# Feature: Command follow ups

## Report

Left by the engine commands track:
- `reindex` names the indexes it would rebuild but runs no online or offline rebuild job and writes no `sys::jobs` row. `reindex_rebuilds_every_index_of_a_relation_as_a_job` and `reindex_of_one_index_clause_rebuilds_it_as_a_job` in `adtb-sim/tests/commands.rs` are ignored until it does.
- `catalog::settings` is not seeded, and nothing in the engine reads a setting (`RetryLimit` and the others).
- `explain analyze` reports actual rows, loops and time only at the root node, and `storage` is empty.
- `pruned` is shown only on index probes and lookups, never on scans or on start-time routing.
- `verify`, `compact`, `print schema`, `pin plan` and the other commands are refused.
