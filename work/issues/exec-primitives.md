---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 721
summary: "the executor lacks the Unicode, regex, zone, period, span, parse and decimal division primitives"
branch: adtb-prim
depends-on:
  - executor
tags:
  - engine
---

# Feature: The executor's missing primitives

## Report

The executor refuses at compile time, or fails at run time:
- `lower`, `upper`, `fold`, `collate`, `grapheme_len`, `matches`;
- `text`, `print`, `parse`, `parse_uuid`;
- the zone functions, `add_period`, the span forms, decimal `div`;
- conversions with rounding, and decimals to floats;
- NFC for `++` and `utf8` when the result is not ASCII-led.

`every_primitive_agrees_with_the_reference` in
`crates/adtb-sim/tests/exec.rs` is the spec for them. Use the crates
already allowlisted for `adtb-reference`: `unicode-normalization`,
ICU4X, `regex`, `jiff`. Keep the implementations independent of
`adtb-reference`'s.
