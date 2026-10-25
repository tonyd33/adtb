---
title: The OS may write back a mapped page before its transaction commits
pick: any
raised-by: [buffer/cache/mmap]
options:
  ro-map:
    title: Map read-only and write only fresh copy-on-write pages with pwrite
    links:
      - {rel: prior-art, to: prior/buffer-mmap}
      - {rel: gains, to: engine/shadow}
  user-copy:
    title: Copy pages to private buffers to modify, write back after the log is durable
  live:
    title: Live with it
    excludes: [buffer/cache/mmap-safety/ro-map, buffer/cache/mmap-safety/user-copy]
---
Under `MAP_SHARED` the kernel flushes a dirty page whenever it likes;
`mlock` pins residency but not write-back (Crotty et al. sec. 2.2).
A page holding a half-done retag (row out of `EmailPass`, not yet in
`Google`) can reach disk with no log record covering it.

## buffer/cache/mmap-safety/ro-map

LMDB's default: the map is `PROT_READ`; a writer allocates new pages,
writes them with `pwrite`, syncs, then flips one of two meta pages.
Stray pointer writes into the map fault. One writer at a time.
http://www.lmdb.tech/doc/group__mdb.html

## buffer/cache/mmap-safety/user-copy

SQLite, MonetDB, RavenDB: a modified page is copied out of the map,
changed, logged, and copied back after commit. Reads still stall on
faults and errors still arrive as `SIGBUS`; the gain over
`buffer/cache/hash-pool` is only the read path.

## buffer/cache/mmap-safety/live

Crash consistency then depends on kernel write-back timing.
