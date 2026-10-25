---
title: A query must be proved to fall inside a partial index's constructor
pick: any
raised-by: [btree/sum-field/ctor-field]
options:
  arm-scoped:
    title: An arm whose pattern names the constructor may use the index
  live:
    title: Live with the index used only for constraint checks
    excludes: [btree/sum-field/implied/arm-scoped]
---
Postgres partial indexes fail when the predicate is parameterized or
written differently. Here the predicate is a constructor test.

## btree/sum-field/implied/arm-scoped

The arm `Paid { ref } if ref == r` is a scan of `invoices` rows with tag
`Paid` and `ref == r`; the index's predicate is exactly the arm's
pattern, so the implication is syntactic and needs no prover.

## btree/sum-field/implied/live

The index enforces `unique` and the planner ignores it.
