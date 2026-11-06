---
title: The durable damage record is bounded by the page that folds it
pick: one
raised-by: [storage/damage-mark/damage-log]
options:
  overflow-database:
    title: Past capacity, record the database's mark and stop recording pages
  chain:
    title: A chain of list pages, allocated as needed
  live:
    title: Live with records lost past capacity
---
A checkpoint folds the damaged pages into a pager-owned list. At 16 bytes per
entry (pid, kind, LSN), an 8 KiB page holds about 500. SQL Server caps
`suspect_pages` at 1,000 rows and stops logging when full (work/kb/sqlserver.md).

## storage/damage-mark/record-full/overflow-database

More damage than a page lists marks the database: the file opens for verify
and dump only, which is what that much damage warrants.

## storage/damage-mark/record-full/chain

Allocation during a checkpoint on a damaged file: the allocator's bitmap may
itself be the damage.

## storage/damage-mark/record-full/live

SQL Server's choice. The page stays refused only until the pager closes.
