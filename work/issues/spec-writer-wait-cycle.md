---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 297
summary: transactions.md §6.3 does not say what happens when two writers wait on each other
branch: adtb-spec3
tags:
  - spec
---

# Bug: Writers waiting on each other

## Report

`docs/spec/transactions.md` §6.3 states writer waits but not a cycle of
them. The engine fails one waiter with a serialization failure, which
§6.1 allows. My ruling: state that a cycle of waits fails one of its
transactions with `SerializationFailure`, which one being a freedom.
