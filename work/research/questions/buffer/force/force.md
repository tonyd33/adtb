---
title: How a commit record is forced to stable storage
pick: one
raised-by: [engine/own-pager]
options:
  fdatasync:
    title: Write then fdatasync on preallocated, zero-filled log segments
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  dsync:
    title: Per-write data sync with O_DSYNC or RWF_DSYNC
    links:
      - {rel: prior-art, to: prior/buffer-linux-io}
  fsync:
    title: Write then fsync
---
`fsync` also flushes inode metadata (size, mtime); `fdatasync` only
what is needed to read the data back, so an append that grows the file
still costs a metadata journal commit. Neither persists a new
directory entry: creating or renaming a log segment needs `fsync` on
the directory. https://man7.org/linux/man-pages/man2/fsync.2.html

## buffer/force/fdatasync

Postgres `wal_sync_method=fdatasync` with `wal_init_zero` and
`wal_recycle`: segments are zero-filled at creation and reused by
rename, so an overwrite inside the segment has no size change and
`fdatasync` flushes only data.
https://www.postgresql.org/docs/current/runtime-config-wal.html

## buffer/force/dsync

Postgres `open_datasync`; `pwritev2(..., RWF_DSYNC)` (Linux 4.7) per
write. With `O_DIRECT` on NVMe this can become a FUA write, one round
trip instead of write plus flush (unverified per driver).

## buffer/force/fsync

Simplest; pays a metadata flush on every commit unless the file system
skips unchanged inodes.
