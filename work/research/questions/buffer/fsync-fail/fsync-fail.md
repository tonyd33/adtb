---
title: What the engine does when a write or fsync fails
pick: one
raised-by: [engine/own-pager]
options:
  panic-replay:
    title: Crash and recover from the log
    links:
      - {rel: prior-art, to: prior/buffer-fsync}
  read-only:
    title: Fail every pending commit and switch the database read-only
  repair:
    title: Repair the failed range from a replica or a local copy
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  retry:
    title: Retry the fsync
    assessment: {grade: broken, why: "ext4, XFS and Btrfs mark pages clean after a failed fsync, so a retry can report success with nothing written"}
---
After a failed `fsync` on Linux the dirty pages are marked clean; on
ext4 and XFS the cache keeps the new bytes, on Btrfs the old. The error
may belong to a write from another descriptor. Rebello et al. (ATC
2020) found no studied system handled it correctly.
https://www.usenix.org/system/files/atc20-rebello.pdf

## buffer/fsync-fail/panic-replay

Postgres since 2018 (backpatched to 9.4): `PANIC`, restart, replay WAL
from the last checkpoint. Correct only if the log itself reached disk
and replay reads what is on disk.
https://wiki.postgresql.org/wiki/Fsync_Errors

## buffer/fsync-fail/read-only

Commits whose log force failed return an error; the pool keeps serving
reads from frames it still trusts; restart needed to write again.

## buffer/fsync-fail/repair

TigerBeetle: a block that fails to write or verify is fetched from a
peer by checksum. Needs redundancy (`engine/distributed` or mirrored
local files).

## buffer/fsync-fail/retry

Pre-2018 Postgres. Unsound on Linux.
