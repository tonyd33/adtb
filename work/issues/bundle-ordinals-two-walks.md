---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "code generation and registration each walk a bundle file to assign ordinals, and already disagree on a declared limit"
tags:
  - quality
  - lang
  - engine
---

# Code generation and registration assign a bundle's ordinals in two separate walks

## Report

`adtb_lang::signature::bundle` (`crates/adtb-lang/src/signature.rs:73-131`) and `adtb::api::registered::checked` (`crates/adtb/src/api/registered.rs:354-449`) each build a bundle's callable list: imported fns first, then queries and transactions, a duplicate-name refusal, a feed and aggregate refusal, and ordinals from 1 (`signature.rs:125`, `registered.rs:440`). The two `imported` functions are parallel copies too (`signature.rs:194`, `registered.rs:502`). Generated `Callable::ORDINAL` values must equal the engine's, and only the fixture test in `crates/adtb-codegen-tests/tests/calls.rs` keeps them so.

They have already drifted: registration refuses a declared `limit` (`registered.rs:394`, `:410`) and `signature::bundle` accepts it, so codegen emits calls for a bundle the engine will not register.

Fix: have `registered::checked` start from `signature::bundle`'s list, adding normalisation and row text per entry, and move the `limit` refusal into `signature::bundle`.
