---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 796
summary: four new too_many_arguments expectations name their arguments instead of what would remove them
closed: 2026-10-26
tags:
  - quality
---

# Feature: argument contexts in place of too_many_arguments expectations

## Report

Each new `#[expect(clippy::too_many_arguments)]` in this range gives a reason that lists its arguments instead of what would have to change for the expectation to go:

- `crates/adtb/src/catalog/moved.rs:157` `held_fills` and moved.rs:310 `merge`: every map function in moved.rs takes the same `a`, `b`, `data` and `out`.
- `crates/adtb/src/plan/lower.rs:69` `plan_in`: the inputs of `plan` plus `bounded`.
- `crates/adtb-lang/src/norm.rs:935` `present_step`: the fields of `Kind::Present`, unpacked.

Fix: a `Mapping { a, b, data, out }` context in moved.rs whose methods are the maps, which removes both expectations there; a plan input struct for `plan` and `plan_in`; pass `Kind::Present`'s fields to `present_step` as the variant's borrowed fields. Where an expectation stays, its reason says what would remove it.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in wqmnkqkx, within the adtb crate and its tests. What remains in adtb-lang is `lang-id-follow-ups`.
