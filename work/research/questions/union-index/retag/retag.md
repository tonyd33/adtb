---
title: Changing a row's constructor moves it between heaps
pick: one
raised-by: [encoding/native-cost/union-index, storage/table-per-ctor]
options:
  forbid:
    title: No in-place retag, constructor changes are delete plus insert
  move:
    title: Retag as an atomic cross-heap move keeping the key
  live:
    title: Live with cross-partition update semantics
---
`update i in identities where i.id == 7 set Google { ... }` on an `EmailPass` row deletes from
one heap and inserts into another. Concurrent readers following the old
version land in the wrong heap; Postgres cross-partition UPDATE has the
same problem. Not researched beyond that analogy (unverified).

## union-index/retag/forbid

The language has no retag. A program deletes the `EmailPass` row and
inserts a `Google` row, with a new or the same id.

```
transact
  let d = delete i in identities where i.id == 7
  let g = insert identities.Google { id: 7, gid: "..." }
  return key g
```

## union-index/retag/move

The engine moves the row within one transaction, keeping its key. How
the union index represents the move under MVCC is `engine/retag-snapshot`.

## union-index/retag/live

Concurrent updaters of the old version get a serialization failure, as
Postgres raises for a row moved to another partition.
