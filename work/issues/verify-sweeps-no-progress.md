---
created-at: 2026-10-30
status: open
kind: bug
priority: 560
summary: "the verify sweeps under writers show no writer did anything, and the crash sweep discards every statement result"
tags:
  - quality
  - test
---

# The verify sweeps under writers pass when no writer got anything done

## Report

Each of these tests asserts that verify found nothing, and none shows the workload ran:

- `crates/adtb-sim/tests/verify.rs:1442` `a_file_after_a_crash_of_writers_and_vacuums_holds_clean` discards every result with `.ok()` at `:1466`, `:1471` and `:1472`. If the insert text stopped parsing, or `vacuum invoices` were refused, or the crash came before the first commit, it verifies a file holding nothing and passes. The test exists to cover large values, vacuum and crash together.
- `:661` `with_writers`, used at `:703` and `:717`, returns no count. `verified` can return before a writer commits, so the run can be a run with no concurrency.
- `:843` `repeated_runs_find_nothing_while_writers_delete_and_vacuum` and `:1331` `a_run_under_writers_of_expression_and_path_indexes_finds_nothing` likewise.

The online verify review saw the effect: removing the vacuum hold left the churn sweep green over 25 seeds, and only a test timed to the index phase caught it.

Fix direction: count completed commits and vacuums in a shared counter. Assert a floor at the end of each test, across the sweep where one seed may legitimately do little. In the crash test keep a failure only where the process is going down: assert no statement failed before the crash, and after reopening assert some invoice survived on at least one seed.
