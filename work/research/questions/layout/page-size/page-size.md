---
title: Page size
pick: one
raised-by: [engine/own-pager]
options:
  p4k:
    title: Fixed 4 KiB pages
  p8k:
    title: Fixed 8 KiB pages
  p16k:
    title: Fixed 16 KiB pages
  variable:
    title: Power-of-two size classes from 64 KiB, chosen per page
    links:
      - {rel: prior-art, to: prior/layout-umbra}
  per-heap:
    title: Fixed per heap, chosen when the heap is created
---
LeanStore (Leis, PVLDB 17, 2024) lists DB2 4 KB, Postgres and SQL
Server 8 KB, MySQL 16 KB, WiredTiger 32 KB, Umbra 64 KB.
https://www.vldb.org/pvldb/vol17/p4536-leis.pdf
Smaller pages cut I/O amplification for point reads; larger ones raise
B-tree fanout and keep more of a value inline.

## layout/page-size/p4k

LeanStore moved from 16 KB to 4 KB for lowest NVMe latency and less
amplification on random workloads. Matches the NVMe logical block, so
one page write is one atomic unit where the device guarantees AWUPF.

## layout/page-size/p8k

Postgres default. A torn 8 KiB write is 16 sectors of 512 bytes.

## layout/page-size/p16k

InnoDB default; 16 KiB is the size Linux RWF_ATOMIC first targets on
XFS and ext4 with 16 KiB blocks (secondary sources).

## layout/page-size/variable

Umbra: one buffer pool, each size class its own virtual range; a leaf
takes the smallest class holding its tuple, so a 1 MiB value is one
1 MiB page and never split across pages.

## layout/page-size/per-heap

A constructor of small fixed rows takes 4 KiB pages, one with large
text 64 KiB, as InnoDB fixes size per instance and DB2 per tablespace
(unverified for DB2). The buffer pool holds several sizes.
