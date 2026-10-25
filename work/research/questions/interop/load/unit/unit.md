---
title: Whether a load is one change set or many
pick: one
raised-by: [interop/load/source, interop/load/command, interop/load/append-api]
options:
  one-set:
    title: The whole load is one change set, checked once at its end
    links:
      - {rel: variant-of, to: write/unit/multi}
  batches:
    title: Each batch commits separately; constraints hold at each batch boundary
    links:
      - {rel: prior-art, to: prior/interop-pg-copy}
  staged:
    title: Rows land in an unconstrained staging relation, then one insert moves them
---
Constraints hold at every statement boundary
(`constraint/timing/multi-assign`). A ten-million-row load either is
one boundary or has many. Many boundaries order the load: a file of
identities cannot commit before the users it references.

## interop/load/unit/one-set

```
transact
  insert users from (...)
  insert identities from (...)   -- FKs, totality, uniques checked here
```

Readers see nothing until the end; an error anywhere aborts all. The
change set is held as heap pages written but uncommitted, not in
memory (`engine/in-heap`).

## interop/load/unit/batches

```
load identities from "identities.csv" batch 100000
```

Each batch is a transaction; a failed batch leaves the earlier ones
committed. Cross-file references need the load ordered by dependency,
and a reference cycle needs one batch.

## interop/load/unit/staged

```
relation staging_identities : IdentityRow key line
insert staging_identities from (from r in read_csv("identities.csv", IdentityRow) select r)
-- inspect, fix with ordinary updates
insert identities from (from r in staging_identities select ...)
```

The staging relation is an ordinary relation whose type is the file's
row type, with only a line-number key. The final insert is
`one-set`.
