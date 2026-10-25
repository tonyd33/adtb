---
title: How index keys are represented and compared
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  memcmp:
    title: Order-preserving byte strings compared with memcmp everywhere
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
      - {rel: prior-art, to: prior/btree-graefe}
  typed:
    title: Fields stored natively and compared by a per-type comparator
    excludes: [engine/fdb]
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  hybrid:
    title: Native keys in leaves, normalized byte strings in branch pages
    links:
      - {rel: prior-art, to: prior/btree-graefe}
---
A key is a tuple of fields, some of them value-level sums. The engine
either encodes it once into bytes whose memcmp order is the key order,
or compares field by field with type knowledge. FoundationDB compares
keys only as bytes; RocksDB and LMDB accept a custom comparator.

## btree/keyenc/memcmp

Graefe's normalized keys; FoundationDB tuple layer, CockroachDB, MyRocks.
Comparison is one `memcmp`; prefix and suffix truncation and ART work on
bytes. Decoding a key back to values must be possible for index-only
scans.

```
key (user_id: Int8 = 3, email: Text = "a@b")
  80 00 00 00 00 00 00 03   61 40 62 00      -- sign-flipped be64, terminated text
```

## btree/keyenc/typed

Postgres opclass support functions, SQLite record compare (serial
types, NULL < numeric < text < blob). Keys stay in row format and
decode for free; every comparison dispatches on type.

## btree/keyenc/hybrid

Graefe: "some systems employ normalized keys in branch nodes but not
in leaf nodes". Branch keys only guide search, so a lossy normalization
(collation weights) is harmless there; leaves keep values.
