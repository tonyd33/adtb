---
title: Physical order and alignment of a constructor's fields
pick: one
raised-by: [storage/table-per-ctor]
options:
  declared:
    title: Declaration order with natural alignment padding
  packed:
    title: Ordered by alignment to remove padding with a field-id to offset map in the catalog
  unaligned:
    title: No padding and unaligned loads
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
---
The language has no positional row order, and fields keep stable ids
through rename and move, so the physical order is the engine's.

```
Order { paid: Bool, id: I64, qty: I16, total: Decimal 2 }   -- Bool 1, I64 8, I16 2, Decimal 16
```

## layout/field-order/declared

```
0 paid  1..7 pad  8 id  16 qty  18..23 pad  24 total   = 40 B
```

## layout/field-order/packed

```
0 total  16 id  24 qty  26 paid  = 27 B (aligned to 8: 32 B)
```

Cap'n Proto places later fields into padding left by earlier ones.
Moving a field into a constructor reassigns offsets; the per-format map
is catalog data. https://capnproto.org/encoding.html

## layout/field-order/unaligned

27 bytes; every load is unaligned (cheap on x86-64 and ARMv8, unverified
for all targets). SQLite records are unaligned and varint-packed.
