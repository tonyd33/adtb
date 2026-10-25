---
title: Which frame the pool evicts
pick: one
raised-by: [buffer/cache/hash-pool, buffer/cache/swizzle, buffer/cache/vmcache]
options:
  clock:
    title: Clock sweep with a small saturating usage count
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  midpoint-lru:
    title: LRU with new pages inserted at an old-sublist midpoint
    links:
      - {rel: prior-art, to: prior/buffer-innodb}
  two-q:
    title: 2Q with a FIFO probation queue and a ghost list
  arc:
    title: ARC balancing recency and frequency lists with ghost hits
  lru-k:
    title: LRU-K evicting by the K-th most recent reference time
  cooling:
    title: Random cooling into a FIFO, re-heat on access
    links:
      - {rel: prior-art, to: prior/buffer-leis}
      - {rel: gains, to: buffer/cache/swizzle}
---
A policy is judged by hit rate and by what a hit costs: anything that
writes shared state on every access (moving a list node, a timestamp)
contends across cores.

## buffer/replace/clock

Postgres since 8.1: pin bumps usage (cap 5); the hand decrements and
takes the first unpinned zero. A hit writes one counter in the
descriptor word it already touches. vmcache's Marked state is clock
with a one-bit count.
https://github.com/postgres/postgres/blob/master/src/backend/storage/buffer/README

## buffer/replace/midpoint-lru

InnoDB: new pages enter at 3/8 from the tail; promoted only if touched
again after 1000 ms. Hits on young pages near the head skip the list
move. https://dev.mysql.com/doc/refman/8.4/en/innodb-performance-midpoint_insertion.html

## buffer/replace/two-q

Johnson and Shasha (VLDB 1994): A1in FIFO for first touches, A1out
ghost ids, Am LRU for re-referenced pages (unverified details).
Postgres 8.0.2 shipped it in place of ARC.

## buffer/replace/arc

Megiddo and Modha (FAST 2003): T1/T2 resident lists, B1/B2 ghost
lists, the split adapts on ghost hits. Postgres removed it in 8.0.2
over a pending US patent.
https://www.postgresql.org/docs/release/8.0.2/

## buffer/replace/lru-k

O'Neil, O'Neil, Weikum (SIGMOD 1993): keep the last K reference times
per page, evict the oldest K-th (unverified details). A hit writes a
timestamp.

## buffer/replace/cooling

LeanStore: when free frames run low, unswizzle random pages into a
FIFO (about 10% of the pool); an access re-swizzles, the FIFO tail is
evicted. Hot hits write nothing.
https://db.in.tum.de/~leis/papers/leanstore.pdf
