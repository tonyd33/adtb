---
title: Layout of a non-unique ctor-set index on an ordered key-value store
pick: one
raised-by: [engine/kv-heaps/key-prefix]
options:
  pk-in-key:
    title: One key per entry with constructor and primary key as its suffix, empty value
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
  posting-value:
    title: One key per index value, the value a list of constructor and primary key
---
On a KV store the entry's TID is the row's primary key
(`engine/kv-heaps/key-prefix`). A unique index maps key to
`(ctor, pk)`; a non-unique one has many rows per key.

## btree/kv-layout/pk-in-key

MyRocks secondary index: 4-byte index id, null flag, secondary key,
primary key, empty value; inserts and deletes are blind writes to
distinct keys, so concurrent inserts of one value never conflict.

```
idx#12 | enc(user_id=3) | ctor uvarint(Google) | enc(pk=7)   ->  ""
```

## btree/kv-layout/posting-value

```
idx#12 | enc(user_id=3)  ->  [(EmailPass, 9), (Google, 7)]
```

Read-modify-write per insert, or a RocksDB merge operator appending to
the list (unverified as used for secondary indexes). Under optimistic
transactions, every inserter of one value conflicts on its key.
