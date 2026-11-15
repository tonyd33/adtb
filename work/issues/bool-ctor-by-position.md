---
created-at: 2026-10-26
status: resolved
closed: 2026-11-14
kind: feature
priority: 765
summary: "file scans build Bool and Option values by constructor position, and three places build Bool literals separately"
tags:
  - quality
  - engine
---

# Feature: build Bool and Option values from file cells by constructor name

## Report

`Shape::Bool` (`crates/adtb-file/src/json.rs:62`) builds the value as `of_ctor(u32::from(*b))`, assuming `False` is constructor 0 and `True` is 1, and `none` and `some` in `crates/adtb-file/src/lib.rs` assume `Option`'s constructors are 0 and 1. The planner and polarity code look these up by name (`ctor_index("True")`). Reordering the prelude declarations silently swaps every boolean read from a file.

Near-duplicate Bool literal builders sit in `crates/adtb/src/plan/lower/split.rs:189-212` and `crates/adtb/src/write/delta/mod.rs:873-886`.

Fix: resolve the constructor index through the schema once, behind one shared helper for Bool and Option values, and use it from all three places.

## 2026-10-27: typed values

`crates/adtb/src/api/typed.rs` (about lines 253 to 291) builds `bool` and `Option` by constructor position too.

## Also by spelling

crates/adtb/src/api/error_value.rs:319 and catalog/values.rs:35 build a Bool from the spellings "True"/"False" (quality review of 2026-11-13).

## Resolved

Resolved by structure-lang-identity steps 9a (qsvnzzws 4385d774) and 9b (xoqlkpzz 50de734f, 2 rounds, full check 0, 3785 tests). Prelude constructor positions are named once in `schema::prelude::ctor` (Bool, Option, Result, Rounding, Overflow, Fault), each checked against the parsed prelude's position and constructor name by `every_prelude_constructor_constant_is_named_for_its_constructor_at_its_parsed_position`; the engine's and the reference's `Val`/`Value` helpers, `Rounding`, `try`, Option reads and `Fault::ctor()` use them (each crate's `each_fault_is_the_prelude_constructor_of_its_name` checks its fault table). Prelude types are reached through `SchemaDefs::prelude()`; `Schema::prelude_type` is removed. structure-check row `prelude_positions` counts what is left (one-constructor sums and verify's two lines, step 9c). Filed beside: error-sums-by-spelling.
