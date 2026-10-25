---
title: "What update x and delete x may name"
pick: one
options:
  snapshot-row:
    title: "x bound to a snapshot row: a generator, an upsert's old, a let of a row read from the snapshot, or a match binder over one; never a row a write produced"
  any-binding:
    title: "Any name bound to a row, a let of a write included"
---
Raised by the implementability review of docs/spec/. writes §3.1
allowed `update x set e` on a row bound by `let`, which may be a row an
insert of the same block only proposes, not a row of the snapshot.

```adt
transact
  let u = insert users { name: n }
  update u set User { u with name: m }   -- refused
```

## spec/writes/bound-row/snapshot-row

`update x` and `delete x` require `x` bound to a row of a relation
read from the statement's snapshot: by a generator of a query, by an
upsert's `old`, by a `let` bound to a row-valued expression over the
snapshot (such as one that follows a key to the row it names), or by a
binder of a match over such a row. A row a write produced is not a
snapshot row: a name bound by `let` from a write is refused by the type
checker.

## spec/writes/bound-row/any-binding

Any row-typed name may be updated or deleted. A row that is only
proposed by the same statement has no pre-state row to delete, so the
meaning of the write is undefined.
