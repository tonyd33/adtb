---
title: What a match compiles to
pick: some
raised-by: [query/construct]
options:
  union-all:
    title: UNION ALL of one select per arm
  outer-join:
    title: Outer joins of every constructor and CASE on the tag
  native:
    title: A native match operator in the planner
    excludes: [substrate/pg-compiler]
---
The relational plan an arm-per-constructor query becomes.

## query/union-all

```
(from e in identities.EmailPass select { id: e.id, out: e.email })
union
(from g in identities.Google select { id: g.id, out: g.gid })
```

Each arm sees only its constructor relation. Ordering and aggregation
over the whole match wrap the union. Postgres LIST partitioning on a tag
is this plan with partition pruning; arm output types must unify.

## query/outer-join

The `query/plain-sql` shape, emitted by the compiler. One scan of the
parent; cost grows with the number of constructors joined. Branch on the
tag, never on field nullness; a constructor table not 1:1 with the
parent multiplies rows.

## query/native

A plan node that dispatches each row to its arm by tag, reading only the
matched constructor's storage. No precedent found; Append over per-tag
scans with pruning is the nearest. Needs per-arm cardinality estimates.
