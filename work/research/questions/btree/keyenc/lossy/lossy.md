---
title: Some orders lose the value in its normalized key
pick: any
raised-by: [btree/keyenc/memcmp]
options:
  unpack-info:
    title: Store the bytes needed to restore the value beside the key
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
  heap-recheck:
    title: Index-only scans refused for lossy fields, values read from the heap
  code-point:
    title: Text keys compare by code point only, so encoding is lossless
  live:
    title: Live with index keys that cannot be decoded
    excludes: [btree/keyenc/lossy/unpack-info, btree/keyenc/lossy/heap-recheck, btree/keyenc/lossy/code-point]
---
A case-insensitive or locale collation maps distinct strings to equal
weights. The normalized key orders correctly but `"Ab"` and `"ab"`
share it.

## btree/keyenc/lossy/unpack-info

MyRocks stores restore data in the value: tag 0x02, 2-byte length,
data. https://github.com/facebook/mysql-5.6/wiki/MyRocks-record-format

## btree/keyenc/lossy/heap-recheck

The planner marks such fields as not returnable from the index.

## btree/keyenc/lossy/code-point

`Text` orders by Unicode scalar value; a collated order is an explicit
key expression, `index Person (collate(name, "de"))`, whose value is
the weight string itself.

## btree/keyenc/lossy/live

Index scans always fetch the row.
