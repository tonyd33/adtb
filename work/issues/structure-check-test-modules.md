---
created-at: 2026-11-07
status: resolved
closed: 2026-11-16
kind: bug
priority: 30
summary: "disallowed_methods rows count matches per file path, so an expect on a #[cfg(test)] module inside a source file counts as prod; crates/adtb-sys/src/pages/mod.rs is the one site"
tags:
  - class-none
---

# structure-check counts an expect inside a cfg(test) module as prod

## Report

## 2026-11-07

Found in review of storage owners step 1. The `disallowed_methods` rows match text per file; the `crate_fields` rows already parse with crates/adtb-structure. Direction: count `expect` attributes through the same parse, classing an item inside a `#[cfg(test)]` module as test; a structure-check-test case shows it.

## 2026-11-16, resolved

Landed as wpomtmmz 8be12b90 (2 rounds). `disallowed_methods` expects are counted through crates/adtb-structure's parse: one `cfg(test)` check over the attributes of every item, statement and expression on the path, `cfg_attr(test, ...)` as test, the lint list parsed by path, macro bodies scanned by token. crates/adtb-sys/src/pages/mod.rs moved from prod to test; no other count changed. Under `--against`, a file's test count may rise by as much as the same row's prod count in that file fell. Main's full check: 3857 of 3858, one timeout under two-track load (conflict-branch-test-near-timeout), which passes alone in 27 s.
