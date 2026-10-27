---
created-at: 2026-10-26
status: open
kind: feature
priority: 769
summary: "vacuum, explain and explain analyze command tests check substrings or accept two errors, and no test mixes a write with a refused command"
tags:
  - quality
  - test
---

# Feature: command tests pin their results

## Report

In `crates/adtb-sim/tests/commands.rs`:

- `vacuum_removes_versions_no_snapshot_sees` (commands.rs:327-336) asserts only that some row lacks the text `removed: 0`, not that `EmailPass` removed exactly 1 and `Google` and `Apple` 0.
- `explain_of_a_read_of_some_constructors_names_the_pruned_ones` (commands.rs:132-145) checks only for `"pruned": [{`, never that `EmailPass` and `Apple` are the ones named.
- `explain_analyze_of_a_write_checks_its_constraints` (commands.rs:228-244) accepts either `Violated` or `KeyViolation`; the `unique (user) in Google` collision has one right answer.
- `a_command_the_engine_does_not_run_is_refused_whole` (commands.rs:418) runs each command alone, so a text holding a write and a refused command applying nothing is never tested.

Fix: assert the exact rows, constructors and error in each, and add the mixed text case.
