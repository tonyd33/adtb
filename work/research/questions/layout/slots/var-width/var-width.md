---
title: Variable-width fields break fixed slots
pick: any
raised-by: [layout/slots/fixed-slot, layout/slots/pax]
options:
  split-row:
    title: Fixed part in the slot and variable bytes in a page-local area addressed by offset
  side-heap:
    title: Variable bytes in a separate per-heap store and the slot holds a reference
  live:
    title: Restrict fixed slots to constructors without variable-width fields
    excludes: [layout/slots/var-width/split-row, layout/slots/var-width/side-heap]
---
`Text`, `Bytes`, value-level sums with unequal payloads and recursive
values have no fixed width.

## layout/slots/var-width/split-row

Umbra's string header: a 16-byte fixed part (u32 length, then 12 bytes
inline or a 4-byte prefix plus an offset into the page's variable area).
Short strings never leave the slot; comparisons often end at the prefix.
Freeing a row leaves a hole in the variable area, compacted in page.

```
slot (fixed): ... | gid: u32 len | u8[4] prefix | u32 off | u32 pad
var area at page end: ... gid bytes ...
```

## layout/slots/var-width/side-heap

InnoDB DYNAMIC moves long columns off-page behind a 20-byte pointer;
here every variable field does. Costs a second page access for each.

## layout/slots/var-width/live

Fixed slots only where the constructor's fields are all fixed width
(`layout/slots/by-schema`).
