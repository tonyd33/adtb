---
created-at: 2026-10-26
status: open
kind: bug
priority: 99
summary: "a debug assert in canon fires on a declaration naming a parenthesised type, such as Option (Key users)"
---

# Canon paren type

## Report

## Report

`crates/adtb-lang/src/canon.rs:104` asserts on `Option (Key users)`, `Set (Key orders)` and `Set (Local {..} | Remote {..})` in a declaration. It blocks 48 conformance files in debug builds; 44 pass in release. Given to the front end track.
