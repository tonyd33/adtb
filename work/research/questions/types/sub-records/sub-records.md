---
title: Whether anonymous result records have width subtyping
pick: one
raised-by: [model/anon-results/anon-records]
options:
  width:
    title: A record with more fields is a subtype of one with fewer
  depth-only:
    title: Same fields, each field covariant
  exact:
    title: Record types are equal or unrelated
---
Result records are structural; stored types are nominal
(`lang/record-poly/nominal`). The question is only about anonymous
records, e.g. a registered query declared to return `{ id: Key users }`
whose body builds `{ id: u.id, name: u.name }`.

## types/sub-records/width

```
query ids () -> Set { id: Key users } = from u in users select { id: u.id, name: u.name }
```

TAPL S-RcdWidth. With set semantics, dropping a field merges rows, so
the subsumption changes cardinality: the coercion is a projection.

## types/sub-records/depth-only

`{ k: Key users.Admin }` <: `{ k: Key users }`; field sets must match.
TAPL S-RcdDepth.

## types/sub-records/exact

No subtyping on result records; a projection is written.
