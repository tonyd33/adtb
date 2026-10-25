---
title: What an index or FK target ranges over
pick: one
raised-by: [encoding/native]
options:
  ctor-set:
    title: Any set of constructors of one sum
  sum-or-ctor:
    title: Only the whole sum or a single constructor
---
Logical scope of an index, unique constraint or FK target, independent
of where rows are stored. A constructor set is a relation variable
together with a set of constructors of its element type, never a set of
constructors of a type alone: one type may have several relation
variables. The sum is the set of all its constructors;
`identities.Google` is a singleton; `identities.(Google | Facebook)` a
subset.

## index/ctor-set

Every index is declared over a constructor set S and holds entries
`(key, ctor, tid)` for rows whose constructor is in S. Under one heap
per constructor it is a union index over the heaps in S; under one heap
for the sum it is a partial index `WHERE ctor IN S`. Sum PK, constructor
`unique` fields and subset FK targets are all instances.

```
relation identities : Identity
  key id                              -- S = all
  unique (gid) in Google              -- S = Google
  index (id) in (Google | Facebook)
```

## index/sum-or-ctor

Two index kinds: sum-wide and per-constructor. Subset FK targets need
nested sums or emulation.
