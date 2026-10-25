---
title: A delete and reinsert of the key looks like a retag to a key relookup
pick: any
raised-by: [mvcc/link/key-relookup]
options:
  in-flag:
    title: The retag's new version carries a retagged-in flag the waiter requires
  live:
    title: Live with a waiter updating a reinserted row as if it were the old one
    excludes: [mvcc/link/reinsert/in-flag]
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
---
T deletes identity 7 and inserts a new `Google` row with id 7; the new
row's xmin equals the old xmax, as after a retag. OrioleDB documents
that a waiting updater then updates the new row
(https://github.com/orioledb/orioledb/blob/main/doc/architecture/row-level-concurrency.mdx).
Only a retag sets the moved marker, which rules out a plain delete,
but a transaction can retag and then delete and reinsert.

## mvcc/link/reinsert/in-flag

One infomask bit, set only on the version a retag inserts and cleared
by nothing. The waiter follows only to a version with the flag and
xmin equal to the old xmax; a reinserted row lacks it.

## mvcc/link/reinsert/live

Accept the OrioleDB behaviour.
