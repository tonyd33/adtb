---
title: How rows are arranged within a constructor heap page
pick: one
raised-by: [engine/own-pager]
options:
  slotted:
    title: Slotted pages with a line pointer array and variable-size tuples
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
      - {rel: prior-art, to: prior/layout-sqlite}
  fixed-slot:
    title: Fixed-size slots addressed by index with no line pointers
  pax:
    title: PAX pages with each fixed field columnar and variable data at the end
    links:
      - {rel: prior-art, to: prior/layout-umbra}
  by-schema:
    title: Fixed slots for constructors with only fixed-width fields and slotted pages otherwise
---
One heap per constructor means one schema per heap: there is no
per-row type, no null bitmap (no NULL), and the row width of a
constructor with only fixed-width fields is a constant of the heap.

```
type Identity
  = EmailPass { id: I64, user: Key users, created: Instant }     -- 8+8+8 = 24 B fixed
  | Google    { id: I64, user: Key users, gid: Text }            -- 16 B fixed + text
```

## layout/slots/slotted

```
page
  header | lp[0] lp[1] ... ->        free        <- ... tuple1 tuple0
  lp = u16 offset, u15 length, flags   (Postgres: 4 bytes)
```

Tuples move during in-page compaction without changing their TID.
Costs 4 bytes per row plus per-row length and alignment. A 24-byte
`EmailPass` row with a 24-byte MVCC header and 4-byte line pointer is
52 bytes, 157 per 8 KiB page.

## layout/slots/fixed-slot

```
page (8 KiB), slot width w = header 24 + body 24 = 48
  0    page header (40 B) + free-slot bitmap (ceil(n/64) * 8 B)
  hdr  slot[0] slot[1] ... slot[n-1]       slot i at hdr + i*w
  n = floor((8192 - 40 - 8*ceil(n/64)) / 48) = 169
```

TID slot number is an array index; no line pointer, no length, no
compaction. Retag and delete free a slot in place.

## layout/slots/pax

Umbra leaves: field `f` of row `i` at `base_f + i * width_f`; variable
data packed from the page end. Scans of one field read one region;
inserts touch every region. Ailamaki et al., VLDB 2001 (unverified
citation).

## layout/slots/by-schema

The heap's schema picks: `EmailPass` gets fixed slots, `Google`
(holding `Text`) gets slotted pages. A schema change adding a `Text`
field converts the heap's format (`layout/heap-format`).
