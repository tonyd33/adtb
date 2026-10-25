---
title: What identifies a schema version across databases
pick: one
raised-by: [surface/version/counter]
options:
  db-counter:
    title: A database id plus the log position
  log-hash:
    title: A hash of the operator log up to the position
  both:
    title: A position for humans, a hash for checking
---
Two databases at version 8 can hold different schemas, so a bundle
checked in CI at 8 says nothing about production at 8, and every
embedded file has its own counter.

## surface/db-counter

`(db 3f2a…, 8)`: equal only for one database.

## surface/log-hash

The version is the hash of operators 1..8; any two databases with the
same log prefix agree.

## surface/both

Shown as `8 (a91c...)`; bundles pin the hash, people read the number.
