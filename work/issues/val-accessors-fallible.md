---
created-at: 2026-10-25
status: open
kind: feature
priority: 595
summary: "the reference interpreter's Val accessors panic on a mistyped value instead of returning an internal error"
tags: [quality]
---

# Feature: fallible Val accessors in the reference

## Report

`truth`, `ctor_of`, `fields`, `as_set` and `int` in `crates/adtb-reference/src/value.rs` (about 84 uses) still panic when a checker bug hands them the wrong value. Give them fallible forms returning `Abort::Internal`, or fallible versions on the case and filter paths, so the reference matches the engine.
