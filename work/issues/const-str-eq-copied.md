---
created-at: 2026-11-15
status: open
kind: bug
priority: 880
summary: "the const fn same comparing two strs at compile time is copied in adtb-lang ids/fixed.rs and adtb catalog/codec.rs"
tags:
  - quality
  - lang
  - engine
  - class-copied-rules
---

# The compile-time str comparison is copied in two crates

## Report

Found by the quality review of reviewed..582d7af5. `const fn same(a: &str, b: &str) -> bool` is written out identically in crates/adtb-lang/src/ids/fixed.rs:578 (for `fixed_ctor` and `fixed_rel`) and crates/adtb/src/catalog/codec.rs:32 (for `codec::field`). 10b will add more compile-time name lookups.

Fix direction: one `pub const fn` in adtb-lang (beside `fixed_ctor`), used by both.
