---
title: How a partially written page is prevented or repaired
pick: one
raised-by: [engine/own-pager]
options:
  fpw:
    title: Log a full page image on the first change after each checkpoint
    excludes: [engine/shadow]
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  doublewrite:
    title: Write each page to a doublewrite area and sync before its home location
    links:
      - {rel: prior-art, to: prior/layout-innodb}
  atomic:
    title: Rely on device atomic writes of one page
    needs-one-of: [buffer/io-path/direct, buffer/io-path/direct-data]
  cow:
    title: Never overwrite a page in place
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
  detect-only:
    title: Detect torn pages by checksum or sector stamps and fail
---
A page larger than the device's atomic unit can be half written at
power loss. Survey: https://transactional.blog/blog/2025-torn-writes

## layout/torn/fpw

Postgres `full_page_writes`; SQLite WAL frames are whole pages. Cost is
log volume right after a checkpoint, roughly one page per first touch.
https://www.postgresql.org/docs/current/wal-reliability.html

## layout/torn/doublewrite

InnoDB: pages written in large sequential batches to
`#ib_<page_size>_<n>.dblwr` with one fsync, then to their home. Every
page written twice; recovery copies an intact version back.
https://dev.mysql.com/doc/refman/8.4/en/innodb-doublewrite-buffer.html

## layout/torn/atomic

Linux `pwritev2(RWF_ATOMIC)` (6.11; XFS and ext4 from 6.13): O_DIRECT,
power-of-two naturally aligned length within the device's atomic
unit (NVMe AWUPF). AlloyDB Omni writes 8 KB pages this way with full
page writes off (secondary sources). https://lwn.net/Articles/974578/

## layout/torn/cow

LMDB, TigerBeetle, WiredTiger: a new location per write, the old copy
valid until the root switch or checkpoint. No repair needed.

## layout/torn/detect-only

SQL Server torn-page detection stamps 2 bits per 512-byte sector;
Oracle repeats the SCN at the page tail. Recovery stops on a torn page
and needs a backup (secondary source).
