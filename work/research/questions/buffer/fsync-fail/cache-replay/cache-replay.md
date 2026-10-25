---
title: Replay after a process crash reads the kernel cache, not the disk
pick: any
raised-by: [[buffer/fsync-fail/panic-replay, buffer/io-path/buffered], [buffer/fsync-fail/panic-replay, buffer/io-path/direct-data]]
options:
  direct-recovery:
    title: Recovery reads the log and pages with O_DIRECT
  reboot:
    title: Treat a write error as needing a host reboot or remount
  live:
    title: Live with it
    excludes: [buffer/fsync-fail/cache-replay/direct-recovery, buffer/fsync-fail/cache-replay/reboot]
---
A process restart keeps the page cache. Pages marked clean after the
failed write still hold bytes that never reached disk; replay reads
them as durable and acknowledges state the disk lacks (Rebello et al.
finding #8, for LevelDB, SQLite, PostgreSQL).

## buffer/fsync-fail/cache-replay/direct-recovery

Open log and data with `O_DIRECT` during recovery only; the cache is
bypassed and stale entries are invalidated by the direct read
(unverified that every file system invalidates).

## buffer/fsync-fail/cache-replay/reboot

The engine refuses to restart in-process after `EIO` on write; an
operator remounts or reboots, dropping the cache.

## buffer/fsync-fail/cache-replay/live

Recovery may confirm unwritten data.
