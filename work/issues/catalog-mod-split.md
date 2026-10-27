---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 862
summary: catalog/mod.rs grew past the 1500 line limit and went on the size allowlist; split it and take it off
branch: catsplit
tags:
  - quality
  - engine
---

# Feature: split catalog/mod.rs

## Report

`crates/adtb/src/catalog/mod.rs` reached 1699 lines with the wstates track and 1701 with bundles, over the 1500 line limit of `work/tools/size-check.sh`, which `check` runs. The plan track also moved `plan/lower.rs` to `plan/lower/mod.rs` without updating `work/tools/size-allowlist`, so `check` failed from then on. Both are on the allowlist now. Split `catalog/mod.rs` by concern (migration, validation, the data checks for disable and remove, unique reports) and remove it from the allowlist. Landing should run `size-check.sh`.
