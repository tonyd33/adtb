---
created-at: 2026-11-14
status: resolved
closed: 2026-11-15
kind: bug
priority: 170
summary: "the Error encoder and the audit tables name prelude constructors of the error and outcome sums by spelling"
tags:
  - engine
  - class-identity-by-spelling
---

# The Error encoder and the audit tables name prelude constructors by spelling

## Report

Left by structure-lang-identity step 9b (xoqlkpzz 50de734f). crates/adtb/src/api/error_value.rs (`value`, `ctor`, about :300-370) names the constructors and fields of the prelude's `Error`, `RetryReason`, `RejectReason` (49 constructors), `FatalReason` and `Limit` by text, and api/server/tables.rs names `Origin`'s and `Outcome`'s constructors by text. Each lookup starts from a prelude `TypeId` and fails loudly on a missing name (a panic in the encoder, `malformed` in tables.rs), so a reorder or rename of the prelude text cannot change a result silently; it is still identity by spelling. Direction: mint their positions in `schema::prelude::ctor` beside the others (checked by the constants test) and pass them.

Defect class: Catalog rows and identity by position or spelling.

## Resolved

Landed as lvyxkqxt e1af4b3f (structure-lang-identity step 11 part c, 2 rounds, full check 0, 3839 tests). Error, origin and outcome values encode by `engine_ctor` fixed ids (72 constants); each constructor's fields are compile-time handles in `SHAPES` (a misspelt field fails the build); `Sums` is built once per engine, checks fields both ways and every part against its declared type. A reversed prelude test moves every constructor and field and checks each value encodes to its parsed text.
