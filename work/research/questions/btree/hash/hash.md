---
title: Hash indexes alongside ordered ones
pick: any
raised-by: [encoding/native-cost/union-index]
options:
  eq-unique:
    title: Hash ctor-set index for equality probes, entries hold hash, ctor and tid
---
Uniqueness and FK probes are equality lookups; an ordered index pays
`log n` for them. Postgres hash indexes store only the 4-byte hash
(lossy), are single-column, `=` only, and cannot enforce uniqueness.
https://www.postgresql.org/docs/current/hash-index.html

## btree/hash/eq-unique

```
unique (id) using hash          -- clause on relation identities
bucket entry: hash u32 | ctor | tid      -- equal hashes recheck the heap row
```

Uniqueness re-checks every equal-hash entry through the heap its
`ctor` names. No ordered scan, so no `merge-append` and no range lock
under SSI; Postgres SSI locks a whole hash bucket (unverified).
