---
title: Data structure of ctor-set indexes on the own pager
pick: one
raised-by: [engine/own-pager]
options:
  blink:
    title: Disk B+-tree with right-links and high keys
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
      - {rel: prior-art, to: prior/btree-graefe}
  bw:
    title: Latch-free Bw-tree with delta chains and a mapping table
    assessment: {grade: costly, why: an optimised reimplementation is slower than a B+tree with optimistic lock coupling}
    links:
      - {rel: prior-art, to: prior/btree-bwtree}
  art:
    title: In-memory adaptive radix tree, not paged
    needs: [btree/keyenc/memcmp]
    links:
      - {rel: prior-art, to: prior/btree-art-olc}
---
Every ctor-set index (sum PK, constructor uniques, subset FK targets)
holds `(key, ctor, tid)` entries over several constructor heaps. On our
own pager the structure is ours to pick. Probes are point lookups
(uniqueness, FK), ordered range scans (`merge-append` sum scans, range
predicates) and the per-sum bulk delete pass.

```
relation identities : Identity
  key id                              -- S = all ctors
  unique (gid) in Google              -- S = {Google}
  index (user) in (Google | Facebook)
```

## btree/structure/blink

Lehman and Yao (TODS 1981) as Postgres nbtree implements it: each page
carries a right-link and a high key; a descent whose key exceeds the
high key moves right, so a split never needs locks on more than one
level at a time. `(ctor, tid)` is the trailing tiebreak attribute, so
every entry is unique as L&Y require.

```
leaf page (8 KiB)
  0     page header (24 B: lsn, checksum, flags, lower, upper, special, ...)
  24    line pointers, 4 B each
  ...   free space
  ...   index tuples, packed from the end
  8176  special (16 B): prev u32 | next u32 | level u32 | flags u16 | cycle u16
item 1 = high key on every non-rightmost page
```

https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/nbtree/README

## btree/structure/bw

Levandoski et al. (ICDE 2013), used in Hekaton: updates prepend delta
records to a node and swap its mapping-table slot by CAS; chains are
consolidated. Wang et al. (SIGMOD 2018) found their optimised
OpenBw-Tree "considerably slower" than ART, Masstree and an OLC
B+tree. A mapping table cannot shrink without stopping writers.

## btree/structure/art

ART (Leis et al. ICDE 2013): Node4, Node16, Node48, Node256 at 52, 160,
656, 2064 bytes; path compression and lazy expansion bound space at 52
bytes per key. Lives in memory; recovery rebuilds it from the heaps.
Keys must be binary-comparable and prefix-free, and non-unique indexes
append the tuple id, here `ctor ++ tid`.

```
key bytes:  enc(user) ++ ctor_be32 ++ block_be32 ++ off_be16
```
