---
title: An open scans the whole retained log, which grows without bound
pick: any
raised-by: [storage/readin-gate/open-map]
options:
  bound-by-recycle:
    title: Recycle segments so the scan is bounded
    links:
      - {rel: same-as, to: storage/readin-gate/recycle}
  fold-at-checkpoint:
    title: Persist the map at checkpoint and scan only from redo
    links:
      - {rel: same-as, to: storage/readin-gate/persisted-map}
  lazy:
    title: Serve reads before the scan finishes, checking a page only once the scan has passed its last record
  live:
    title: Live with open time linear in the log
    excludes: [storage/readin-gate/open-scan-cost/bound-by-recycle, storage/readin-gate/open-scan-cost/fold-at-checkpoint, storage/readin-gate/open-scan-cost/lazy]
---
`read_log` is sequential, single-threaded, 64 KiB direct reads with a
per-block copy and full record decode. On the benchmark's 98 MiB log that is
on the order of the device's sequential read time plus decode; the decode
cost is not measured.

## storage/readin-gate/open-scan-cost/bound-by-recycle

Recycling bounds the log to what the checkpoint and backups pin.

## storage/readin-gate/open-scan-cost/fold-at-checkpoint

The map lives in the file; open replays only from redo.

## storage/readin-gate/open-scan-cost/lazy

A read-in of a page waits until the background scan has passed the page's
last record, which is unknown until the scan ends: in effect every read-in
waits for the whole scan.

## storage/readin-gate/open-scan-cost/live

Open time grows with total writes; the benchmark already holds ten times its
file in log.
