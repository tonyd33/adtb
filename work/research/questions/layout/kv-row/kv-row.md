---
title: How a constructor row maps onto keys and values of an ordered store
pick: one
raised-by: [engine/kv-heaps/key-prefix]
options:
  row-per-key:
    title: One key per row with the whole row as its value
    links:
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
  family-per-key:
    title: One key per group of fields
  field-per-key:
    title: One key per field
    links:
      - {rel: prior-art, to: prior/datomic}
---
The heap of `identities.Google` is the key range under
`(sum, ctor)`; keys are order-preserving tuples so a range scan is the
heap scan.

```
key:    0x15 <relation identities>  0x15 <ctor Google>  0x15 <pk 7>    -- tuple-layer ints, 2 to 9 B each
value:  row body in the constructor's format, no MVCC header (the store versions keys)
```

FoundationDB tuple layer type codes lead each element, so concatenated
tuples sort element-wise. https://github.com/apple/foundationdb/blob/main/design/tuple.md
Short prefixes come from a directory mapping names to small integers.

## layout/kv-row/row-per-key

Record Layer: one protobuf value per record, split above about 100 KB.
A point read is one get; any field update rewrites the row.

## layout/kv-row/family-per-key

CockroachDB column families: `(..., pk, family_id) -> fields` so a hot
counter field is written without the large text beside it (unverified
details).

## layout/kv-row/field-per-key

`(..., pk, field_id) -> value`. Rows of a heap are adjacent keys; reading
a row is a short range scan; prefix compression in the LSM absorbs the
repeated key prefix (unverified for every store).
