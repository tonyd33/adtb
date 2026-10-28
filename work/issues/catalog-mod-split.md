---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 862
summary: catalog/mod.rs grew past the 1500 line limit and went on the size allowlist; split it and take it off
closed: 2026-10-27
tags:
  - quality
  - engine
---

# Feature: split catalog/mod.rs

## Report

`crates/adtb/src/catalog/mod.rs` reached 1699 lines with the wstates track and 1701 with bundles, over the 1500 line limit of `work/tools/size-check.sh`, which `check` runs. The plan track also moved `plan/lower.rs` to `plan/lower/mod.rs` without updating `work/tools/size-allowlist`, so `check` failed from then on. Both are on the allowlist now. Split `catalog/mod.rs` by concern (migration, validation, the data checks for disable and remove, unique reports) and remove it from the allowlist. Landing should run `size-check.sh`.

## 2026-10-27: resolved

`23024b1b` moves migration application and helpers into catalog/migration.rs. Catalog/mod.rs is 1033 lines and removed from the size allowlist. All 18 moved function bodies are unchanged.

Independent review found no blockers. Combined main check exited 0:
2329 root tests passed (8 skipped), 27 campaign tests passed (1 skipped),
root/campaign clippy and fmt, dependency policy and size checks passed.
House checks passed. Evidence: target/integrated-maintenance-check.log
and work/briefs/review-batch-final.md.

## 2026-10-27: resolved
