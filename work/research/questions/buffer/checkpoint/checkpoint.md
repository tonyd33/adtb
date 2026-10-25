---
title: How checkpoints bound recovery and let the log be truncated
pick: one
raised-by: [engine/own-pager]
options:
  sharp:
    title: Quiesce writers, flush every dirty page, log the checkpoint
  spread:
    title: Flush pages dirty at checkpoint start over the interval, redo from its start
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  fuzzy:
    title: Log the dirty page table, redo from its oldest recLSN, flush continuously
    links:
      - {rel: prior-art, to: prior/buffer-innodb}
      - {rel: gains, to: engine/aries}
  cow-snapshot:
    title: Write a consistent snapshot of changed pages to new locations, switch a root
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
      - {rel: gains, to: engine/logical-redo}
---
Recovery starts at a redo point; log before it can be recycled.
Torn-page images (`layout/torn/fpw`) are logged on the first change
after each checkpoint, so checkpoint frequency sets their volume.

## buffer/checkpoint/sharp

Stop writes, flush, log `checkpoint(lsn)`, resume. Redo starts at that
LSN. A pause proportional to dirty pages.

## buffer/checkpoint/spread

Postgres: log a redo record, write every page dirty at that moment
paced to finish at 0.9 of `checkpoint_timeout` (5 min) or before
`max_wal_size` (1 GB), fsync each file, log the checkpoint record
pointing at the redo record, update the control file.
https://www.postgresql.org/docs/current/wal-configuration.html

## buffer/checkpoint/fuzzy

ARIES (Mohan et al. TODS 1992), InnoDB: each dirty frame carries
`rec_lsn`, the LSN that first dirtied it. A checkpoint writes the
dirty page table; redo starts at its minimum. Background flushing
moves the minimum forward with no pause.

```
dirty page table entry
  u32 file   u32 page   u64 rec_lsn
checkpoint record
  u64 redo_lsn = min(rec_lsn)   u32 n   entry[n]   active txns
```

## buffer/checkpoint/cow-snapshot

WiredTiger, TigerBeetle: a checkpoint writes changed pages to fresh
blocks and atomically switches a root (superblock, 4 copies in
TigerBeetle); the log replays from it. No torn-page problem for
checkpointed pages.
https://source.wiredtiger.com/develop/arch-checkpoint.html
