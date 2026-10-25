---
title: Whether page and log I/O goes through the OS page cache
pick: one
raised-by: [engine/own-pager]
options:
  buffered:
    title: Buffered pread and pwrite through the page cache
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  direct:
    title: O_DIRECT for data and log, the pool is the only cache
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
      - {rel: prior-art, to: prior/buffer-linux-io}
  direct-data:
    title: O_DIRECT for data pages, buffered log appends
---
With buffered I/O a page lives twice in RAM (pool and kernel), and
write-back errors surface late through `fsync` (fsyncgate). With
`O_DIRECT` the pool must do its own read-ahead and write-behind.

## buffer/io-path/buffered

Postgres default: `shared_buffers` about 25% of RAM, the kernel caches
the rest. Read-ahead and write coalescing for free; a failed write-back
leaves the page clean in the cache (Rebello et al. ATC 2020).
https://www.postgresql.org/docs/current/runtime-config-resource.html

## buffer/io-path/direct

TigerBeetle, InnoDB `O_DIRECT`, Postgres `debug_io_direct=data,wal`.
Errors return from the write itself. `O_DIRECT` does not flush the
device's volatile cache: commits still need `fdatasync` or FUA. Can
target a raw block device (TigerBeetle).
https://man7.org/linux/man-pages/man2/open.2.html

## buffer/io-path/direct-data

InnoDB's usual setting: data `O_DIRECT`, redo log buffered. Small
commit appends need no alignment; the log takes fsyncgate exposure.
