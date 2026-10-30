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

## 2026-10-30: commands inside a transaction

`command::outside_transaction` refuses `verify`, `vacuum` and `reindex` in an open transaction and in text that writes. `set setting`, `explain` and the bundle commands still run in one, which departs from catalog.md 9.1. Ruling: apply the refusal to every command of 9.1's table; a test pins `set setting` in a transaction and changes with it.
