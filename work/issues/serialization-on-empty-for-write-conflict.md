---
created-at: 2026-11-08
status: open
kind: bug
priority: 200
summary: "a serialization failure on a write conflict reports `on` as empty, which RetryReason documents only for a relation not known, though the conflicting relation is known"
tags:
  - class-failure-flattened
---

# serialization failure on a write conflict reports no relation

`RetryReason::Serialization { txn, on }` documents `on` as the constructor relation of the caller's read or write on the conflicting edge, or `""` when it is not known. Every write conflict the isolation and SSI tests reach over `elems` and `counters` (adtb-sim tests execution/isolation.rs and ssi.rs, run through `adtb_simkit::elle`) reports `on: ""`, though the write names its relation.

The tests match `Serialization { txn, .. }` until this is fixed; the fix asserts the whole `Error` there.
