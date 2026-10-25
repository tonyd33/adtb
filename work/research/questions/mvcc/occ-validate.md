---
title: How commit-time validation checks reads and scans
pick: one
raised-by: [engine/occ]
options:
  read-set:
    title: Re-read every version read and rescan every range scanned
    links:
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
  precision:
    title: Test the writes of transactions that committed meanwhile against logged read predicates
    links:
      - {rel: prior-art, to: prior/mvcc-hyper}
---
Validation cost scales with the read set under one option and with
concurrent writes under the other.

## mvcc/read-set

Hekaton: read stability by checking each read version is still current,
phantom avoidance by repeating scans before commit. A long scan of a
sum repeats a merge over every constructor heap it read.

## mvcc/precision

HyPer precision locking: reads log predicates per relation (64-bit
comparison summaries per attribute); at commit, each version created,
deleted or updated (before and after image) by a transaction that
committed since the reader began is tested against them. Here the
constructor is a predicate attribute: a read of
`identities.(Google | Facebook)` logs `ctor in {Google, Facebook}`, so an
`EmailPass` write fails the test on a tag compare, and a retag is
caught by its after image.

```
predicate log, relation identities:
  ctor in {Google, Facebook} and user = 3
recent write: retag 7 EmailPass -> Google, user 3
  before: ctor EmailPass  (no match)   after: ctor Google, user 3  (match: abort)
```
