---
title: With constructor keys a retag changes the row's identity
pick: any
raised-by: [model/tagged-key]
options:
  rewrite-referrers:
    title: A retag rewrites every reference to the row in the same transaction
  new-row:
    title: A retag is a new row and referrers follow the FK action
    links:
      - {rel: same-as, to: fk/retag/delete-insert}
  live:
    title: Live with sums whose constructors need not share a key
    excludes: [model/key-retag/rewrite-referrers, model/key-retag/new-row]
---
`EmailPass "a@b"` becoming `Google "123"` has a new sum key, so every
`Key identities` pointing at it holds a stale value. `fk/retag/recheck`
assumes the reference survives the retag.

## model/key-retag/rewrite-referrers

ON UPDATE CASCADE over the sum key, across every referring relation.

## model/key-retag/new-row

`fk/retag/delete-insert` was defeated for firing ON DELETE on a row that
still exists; this is that shape again.

## model/key-retag/live

A sum that is retagged declares a shared key; `tagged-key` suits sums
whose rows never change constructor.
