---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 701
summary: "some migration operands get no resolved names, and an unresolved name prints unqualified unnoticed"
closed: 2026-10-25
---

# Feature: close the canonical term gaps

## Report

`Logged::names` holds no names for a `script` write, a grant or revoke target, or a `revert` fill (scripts go through `check_adhoc`; recording there changes a kernel.md entry point). The forward-reference fallback errors when the operand text occurs more than once in the result, which short operands such as a lone `Text` can hit. A base type written `prelude::I64` keeps its qualifier. Nothing checks that every schema-object name in an operand was resolved: add a check, at least in debug builds and tests, that every identifier `canonical_term` prints bare is a prelude name or a local.

## 2026-10-25: review repro

Worse than a lone `Text`: a first migration with `add type Device = { id: Key devices, ... }`, then `add unique (name) in users`, then `add relation devices ...` makes `Logged::names` fail with "no declaration holds this operand's text" at `name`, so `catalog.migrate` refuses a valid migration. `locate` and `differing` match operands by spelling; record the operand's position when the operator copies it instead.

## 2026-10-25: resolved

## 2026-10-25: resolved

Operators build copied text as `Written` with the migration ranges it copies, and `Logged::names` reads names from those marks; nothing is matched by spelling, and the review repro passes. Scripts resolve through `check_adhoc_recording`, grants and revert fills are named, and `Ref::BuiltinType` records base types. `canonical_term` debug-checks uppercase and qualified names. Left: lowercase names (`canon-lowercase-check`), and `prelude::I64` is refused by the schema today.
