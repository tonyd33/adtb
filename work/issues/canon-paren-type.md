---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 99
summary: "a debug assert in canon fires on a declaration naming a parenthesised type, such as Option (Key users)"
closed: 2026-10-26
---

# Canon paren type

## Report

## Report

`crates/adtb-lang/src/canon.rs:104` asserts on `Option (Key users)`, `Set (Key orders)` and `Set (Local {..} | Remote {..})` in a declaration. It blocks 48 conformance files in debug builds; 44 pass in release. Given to the front end track.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrvrlkrt.
With it and newtype-migration-parse, 51 conformance files passed on the engine and left the failing list, which now holds 5.
