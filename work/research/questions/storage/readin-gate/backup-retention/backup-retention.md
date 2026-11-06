---
title: Recycling drops the evidence for pages whose last record is old
pick: any
raised-by: [storage/readin-gate/recycle]
options:
  pin-oldest-backup:
    title: Keep segments back to the start of the oldest backup that has not been superseded
  persist-before-recycle:
    title: Before a segment is recycled, fold the last LSN of each page it names into a persisted map
    links:
      - {rel: same-as, to: storage/readin-gate/persisted-map}
  live:
    title: Live with lost writes undetected once their log is recycled
---
A page written once long ago has its last record in an old segment. Once
that segment goes, no gate knows the page's expected LSN, and its lost write
is undetectable. catalog.md §11 already says so; today nothing is recycled,
so the case never arises. Backup copies log only from redo
(storage/backup.rs), so the restored file never had the evidence either.

## storage/readin-gate/backup-retention/pin-oldest-backup

Bounds the log by backup cadence, not by checkpoint interval. Backups are
deployment state; the engine needs a registry of which it must serve.

## storage/readin-gate/backup-retention/persist-before-recycle

The persisted map, filled only from what recycling removes: recycle and the
map together give bounded open and full coverage.

## storage/readin-gate/backup-retention/live

What catalog.md §11 states today.
