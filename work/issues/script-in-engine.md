---
created-at: 2026-10-27
status: open
kind: feature
priority: 877
summary: "the engine refuses every migration holding a script, because catalog replay cannot run a write"
tags:
  - engine
---

# Feature: run script operators in the engine

## Report

`script` resolves to `Script { write }`, but `recordable` in `crates/adtb/src/catalog/mod.rs` refuses any migration that holds one: catalog replay keeps rows in memory, and a write needs the `Database` and `Stmt` machinery above the catalog layer. Run a script's write on the state the operators before it produce, inside the migration's transaction. The ignored test `a_script_writes_on_the_state_the_operators_before_it_produce` in `crates/adtb-sim/tests/oplog.rs` states it.
