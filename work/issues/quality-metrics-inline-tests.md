---
created-at: 2026-10-25
status: open
kind: bug
priority: 95
summary: quality-metrics counts panics in inline test modules as non-test code
tags:
  - quality
  - tooling
---

# quality-metrics counts inline test modules as non-test code

## Report

`is_test` in `work/tools/quality-metrics.sh:35` treats a file as a test only by path (`*/tests/*`, `tests.rs`, `*_tests.rs`), so the `#[cfg(test)] mod tests` inside a source file counts as non-test code in the panic/unwrap/expect and new-`pub` sections. On `reviewed..lvzoynyt` the `adtb-sys: +1` it reports is `m.optimistic(1).unwrap()` in the test module of `crates/adtb-sys/src/pages.rs:884`.

Fix: skip added lines after a `#[cfg(test)]` line in the file at the head, or report inline test modules separately.
