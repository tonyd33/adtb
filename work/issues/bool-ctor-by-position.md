---
created-at: 2026-10-26
status: open
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
