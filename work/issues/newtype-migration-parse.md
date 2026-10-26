---
created-at: 2026-10-26
status: open
kind: bug
priority: 99
summary: add type T = new I64 followed by another operator reads the next add and fails to parse
---

# Newtype migration parse

## Report

## Report

`add type Cents = new I64` then another operator fails with "expected a schema operator, found `type`". Conformance files model/04-newtypes, model/09-equality-types, model/10-newtype-key-order. Given to the front end track.
