---
title: How a value-level sum is encoded as an index key component
pick: one
raised-by: [sum/value]
options:
  tag-payload:
    title: Constructor tag, then that constructor's fields in order
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
      - {rel: gains, to: lang/sum-order/derived}
  eq-hash:
    title: Hash of a canonical encoding, equality probes only
    links:
      - {rel: gains, to: lang/sum-order/eq-only}
---
A field of sum type in a key (`model/sum-valued-key`, or an index on
`status`) must encode so that equal values give equal bytes and, if
the sum is ordered, byte order is value order. Rows of one constructor
heap have a fixed schema, but a sum-typed field inside them is a tag
plus a payload whose shape depends on the tag.

```
type Status = Draft | Sent { at: Time } | Paid { at: Time, ref: Text }
type Invoice = { status: Status }
relation invoices : Invoice
  key id
  index (status)
```

## btree/sum-key/tag-payload

The FoundationDB tuple layer is this shape for its own type union: a
type code byte, then a payload whose length the code determines, so
elements are prefix-free and order by code first. Each constructor
fixes its field list, so a payload of prefix-free fields is prefix-free;
nested sums recurse; optionality is an explicit constructor, so "nulls
first" is just tag order.

```
Draft             -> 01
Sent { at: t }    -> 02 | be64(t ^ 2^63)
Paid { at, ref }  -> 03 | be64(at ^ 2^63) | ref 00
```

A scan of `status` between `Sent {..}` and `Paid {..}` is the byte
range `[02, 04)`; equality on the tag alone is the prefix `02`.

## btree/sum-key/eq-hash

Under `lang/sum-order/eq-only` no order is promised, so the key may be
`hash64(canonical bytes)` plus recheck in the row. Fixed 8 bytes,
however deep the sum. No range or prefix scan by constructor.
