---
created-at: 2026-11-11
status: open
kind: bug
priority: 520
summary: "the doc comments of Database::abort and Session::abort say the abort fails only when the database is quarantined, while the record also fails on an I/O failure and a closed engine"
tags:
  - quality
  - engine
  - class-none
---

# The abort doc comments state a narrower failure than the record has

## Report

`Session::abort` (crates/adtb/src/write/session.rs:110-111) and `Database::abort` (crates/adtb/src/write/mod.rs:1326-1329) say the abort fails only if the database is quarantined. The abort's commit-log record also fails on an I/O failure (`Storage`, nothing marked) and on a closed engine (`Rejected Closed`), as mvcc.md §4.5 states for `abandon`. A caller reading the doc comment handles one case of three.

Direction: state every way the record can fail, as a postcondition.

Defect class: none.
