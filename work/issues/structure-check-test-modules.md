---
created-at: 2026-11-07
status: open
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
