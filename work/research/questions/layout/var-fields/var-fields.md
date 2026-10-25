---
title: How a row locates its variable-width fields
pick: one
raised-by: [storage/table-per-ctor]
options:
  offset-table:
    title: An array of end offsets after the fixed part, one per variable field
  lengths:
    title: Length-prefixed values in sequence
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
      - {rel: prior-art, to: prior/layout-innodb}
  fixed-headers:
    title: A fixed 16-byte header per variable field holding length and an inline prefix or an offset
    links:
      - {rel: prior-art, to: prior/layout-umbra}
---
Fixed fields sit at offsets known from the heap's schema. Variable ones
(`Text`, `Bytes`, sum payloads under `layout/vsum/tag-var`) need a rule.

## layout/var-fields/offset-table

```
row:  fixed fields | u16 end[k] | var bytes
field j spans [end[j-1], end[j]), end[-1] = start of var bytes
```

O(1) access to any field; 2 bytes per variable field.

## layout/var-fields/lengths

SQLite serial types and InnoDB's reversed length list: reaching field
j decodes j lengths. Compact for few variable fields.

## layout/var-fields/fixed-headers

Umbra: u32 length then 12 bytes inline, or a 4-byte prefix and an
offset. Rows become fixed width with variable bodies elsewhere
(`layout/slots/var-width/split-row`). Comparisons of short or
prefix-distinct strings touch no body.
