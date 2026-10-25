---
title: The unit an undo record restores
pick: one
raised-by: [engine/undo-store]
options:
  row-undo:
    title: Per-row undo records chained from a roll pointer in the row header
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  block-undo:
    title: Per-block undo with a transaction list in each page, readers clone pages as of their snapshot
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
---
In-place heaps keep the newest version; old ones are rebuilt from undo.
A retag cannot be in place: the old heap's row is delete-marked with its
full before-image in undo, the new heap gets an insert record.

## mvcc/row-undo

InnoDB: `DB_TRX_ID` (6 bytes) and `DB_ROLL_PTR` (7 bytes) per row,
insert undo dropped at commit, update undo kept for snapshots. OrioleDB
chains tuple headers from the page into undo. Per row in a constructor
heap:

```
  0  u48 trx_id
  6  u56 roll_ptr     -- undo segment, page, offset
 13  u8  flags        -- delete mark, retagged-out
 14  fields at catalog offsets
```

## mvcc/block-undo

Oracle: an ITL in each block header lists transactions with rows there;
rows point at their ITL slot; a reader builds a consistent-read clone of
the block from undo. Cheap for scans of one constructor heap, costly
for point reads of a hot block. OrioleDB also has block-level records
for compaction, split and merge.
