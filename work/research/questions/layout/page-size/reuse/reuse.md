---
title: Reusing disk space across page sizes can expose stale bytes as an LSN
pick: any
raised-by: [layout/page-size/variable, layout/page-size/per-heap]
options:
  same-size:
    title: Disk space is reused only for pages of the same size
  zero-first:
    title: Space is zeroed or its new header logged before a smaller page reuses it
  live:
    title: Live with it
    excludes: [layout/page-size/reuse/same-size, layout/page-size/reuse/zero-first]
---
Umbra: a 128 KiB page freed and its space reused for two 64 KiB pages;
after a crash where only log records reached disk, recovery reads the
second page's LSN out of the old page's bytes.
https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf

## layout/page-size/reuse/same-size

Umbra's rule: free lists per size class. Space freed in one class is
never usable by another, so a workload shift strands space.

## layout/page-size/reuse/zero-first

Write and sync the new page images, or a log record marking the range
as freshly formatted, before any record names the new pages.

## layout/page-size/reuse/live

Only correct when recovery never reads a page LSN, as with copy-on-write.
