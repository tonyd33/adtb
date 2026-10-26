---
created-at: 2026-10-25
status: open
kind: feature
priority: 709
summary: "only the 10 declaring operators have a resolved form"
---

# Feature: resolved forms for the remaining operators

## Report

`Source::resolved` refuses every operator outside the 10 declaring ones (about 40), so a migration using one cannot be logged. Give each a resolved form and an arm in `catalog/oplog.rs` `operator`. Includes `functions` rows for stored views, and Wasm function and `wasm_modules` rows once `install` lands.
