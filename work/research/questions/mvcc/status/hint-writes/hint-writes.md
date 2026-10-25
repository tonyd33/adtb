---
title: Hint bits make reads write pages
pick: any
raised-by: [mvcc/status/hint-bits]
options:
  vacuum-sets:
    title: Only writers and vacuum set hints, readers keep a per-process xid status cache
  checksum-excludes:
    title: The page checksum skips the hint bits, so a hint-only change is never logged
  live:
    title: Live with hint writes and their full-page images
    excludes: [mvcc/status/hint-writes/vacuum-sets, mvcc/status/hint-writes/checksum-excludes]
    assessment: {grade: costly, why: a scan after a bulk load rewrites every page it reads}
---
A hint dirties the buffer; piecemeal index fetches can write one page
many times. With data checksums every hint-only change is WAL-logged,
and the first after a checkpoint is a full page image
(https://www.postgresql.org/docs/current/runtime-config-wal.html).

## mvcc/status/hint-writes/vacuum-sets

Readers resolve status through the clog plus a small cache of recent
xids; the batched per-sum vacuum stamps hints and freezes in the same
pass, when it writes the page anyway.

## mvcc/status/hint-writes/checksum-excludes

The checksum is computed with the infomask hint bits masked, so a torn
write differing only in hints still verifies; the hint change needs no
WAL. No precedent found (unverified as a known design).

## mvcc/status/hint-writes/live

Postgres behaviour.
