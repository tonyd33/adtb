---
title: Platforms the engine supports
pick: one
raised-by: [substrate/new-engine]
options:
  linux-only:
    title: Linux only
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  linux-first:
    title: Linux tuned, other systems supported with slower paths
  portable:
    title: Linux, macOS and Windows on equal footing
---
Durability primitives differ: `O_DIRECT` (Linux), `F_NOCACHE` (macOS),
`FILE_FLAG_NO_BUFFERING` (Windows); `fsync` on macOS does not flush
the drive cache, `F_FULLFSYNC` does (unverified); FreeBSD re-dirties
pages after a failed write-back, Linux does not (Rebello et al.).

## buffer/platform/linux-only

TigerBeetle-style for servers; rules out an embedded library on
desktop and mobile (`engine/embedded`).

## buffer/platform/linux-first

Postgres: `debug_io_direct` maps to each system's flag; `io_uring` is
one `io_method` among several.

## buffer/platform/portable

SQLite: a VFS layer with per-OS sync rules; fastest paths unused.
