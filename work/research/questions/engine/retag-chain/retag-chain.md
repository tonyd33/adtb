---
title: What a writer waiting on a row sees when the row is retagged
pick: one
raised-by: [union-index/retag/move]
options:
  forward-pointer:
    title: The old version points at the new one by constructor and TID
    links:
      - {rel: prior-art, to: prior/engine-pg-row-movement}
  conflict-abort:
    title: The waiter gets a serialization failure, as for any concurrent update
    excludes: [engine/opt-read-committed]
---
T1 retags identity 7 from `EmailPass` to `Google`; T2 is blocked
updating `identities.EmailPass` where `i.id == 7`. Postgres cannot follow a
row into another partition and raises an error; that behaviour was
defeated for retag as a semantic restriction.

## engine/retag-chain/forward-pointer

The old tuple's successor link is `(ctor, tid)`, so a waiter follows it
across heaps exactly as within one. A writer whose statement targets
`identities.EmailPass` re-evaluates on the successor and finds it no
longer qualifies; one targeting `identities` updates it.

## engine/retag-chain/conflict-abort

Under snapshot or serializable isolation every concurrent update of a
written row aborts the second writer, so retag is no different. Only a
read-committed level, which re-evaluates on the newest version, would
make retag the odd one out.
