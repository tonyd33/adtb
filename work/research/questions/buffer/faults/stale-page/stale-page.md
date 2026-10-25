---
title: A lost write leaves a valid old page that a self checksum accepts
pick: any
raised-by: [buffer/faults/corrupt-detect, buffer/faults/repair]
options:
  parent-checksum:
    title: Every reference carries the checksum of the page it names
    links:
      - {rel: same-as, to: layout/checksum/in-parent}
  lsn-check:
    title: Redo and reads compare the page LSN with the LSN the log or referrer expects
  live:
    title: Live with undetected lost writes
    excludes: [buffer/faults/stale-page/parent-checksum, buffer/faults/stale-page/lsn-check]
---
The device acknowledges a write and drops it, or writes it elsewhere;
the old page at the home location still checksums, names its own pid,
and is served. A heap page that lost a retag's insert is consistent
with itself and inconsistent with the union index.

## buffer/faults/stale-page/parent-checksum

TigerBeetle `(address, checksum)` references; for heaps reached by
TID see `layout/checksum/no-parent`.

## buffer/faults/stale-page/lsn-check

Redo already compares `page_lsn` with each record's LSN; a page whose
LSN is older than a record that should have been applied and flushed
before the checkpoint reveals a lost write. Online: union index
entries could carry the heap page LSN at insert (unverified as
practical). Oracle `DB_LOST_WRITE_PROTECT` logs block versions read
for this (unverified).

## buffer/faults/stale-page/live

Silent divergence between heaps and union indexes.
