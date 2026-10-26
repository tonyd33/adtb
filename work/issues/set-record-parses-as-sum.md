---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 94
summary: "Set { a: I64 } without parentheses parses as a one-constructor anonymous sum named Set, and the error prints both types the same"
closed: 2026-10-26
tags:
  - lang
  - spec
---

# Bug: Set { a: I64 } is read as a constructor named Set

## Report

`Set { a: I64 }` parses as an anonymous constructor `Set` with field `a`, not as `Set` applied to a record type. It prints like the set type, so the error reads "expected Set { a: I64 }, found Set { a: I64 }". Needs a ruling on the grammar (a type argument stop rule exists for clause words and calls): either `Set {` begins a type application, or prelude type names cannot be anonymous constructor names; through the design space.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrvrlkrt.
The ruling is point `spec/anon-sum/prelude-head`; a declared type may still name a constructor `Set`.
