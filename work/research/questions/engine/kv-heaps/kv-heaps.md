---
title: Heaps, TIDs and vacuum have no direct counterpart on an ordered KV store
pick: any
raised-by: [engine/lsm-kv, engine/cow-kv, engine/fdb]
options:
  key-prefix:
    title: A constructor's heap is a key prefix, index values name constructor and primary key
    links:
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
  live:
    title: Live with re-reading the settled storage rulings
    excludes: [engine/kv-heaps/key-prefix]
---
The rulings speak of a heap per constructor, TIDs in union index
entries, and batched vacuum per sum. A KV store has keys, versions and
compaction.

## engine/kv-heaps/key-prefix

Heap for `identities.Google` = keys under `(identities, ctor_id(Google))`,
contiguous, so dropping a constructor's rows is a range delete. A union
index entry's TID becomes the primary key, so the entry is
`(key) -> (ctor_id, pk)`; retag rewrites the row key and the entry's
constructor. Vacuum is compaction plus tombstones; batching per sum is
moot. Union index entries stay scattered by key, as with heaps. FDB
Record Layer's record type key prefix is the same layout.

```
update i in identities.EmailPass where i.id == 7 set Google { gid: "x" }
-- delete (identities, EmailPass, 7); put (identities, Google, 7)
-- put (identity_pk, 7) -> (Google, 7)
```

## engine/kv-heaps/live

Treat the settled vocabulary as describing a pager and revisit it.
