---
title: An in-place index update on retag hides the row from older snapshots
pick: any
raised-by: [union-index/retag/move]
options:
  two-entries:
    title: Retag adds an entry for the new version, the old one stays until vacuum
    links:
      - {rel: prior-art, to: prior/engine-pg-unique}
  entry-history:
    title: The entry keeps the previous constructor and TID beside the current one
  live:
    title: Live with older snapshots missing retagged rows in index probes
    excludes: [engine/retag-snapshot/two-entries, engine/retag-snapshot/entry-history]
    assessment: {grade: broken, why: a snapshot reader sees a row vanish from a key it can see in the heap}
---
The accepted move says the entry is updated in place to `(7, Google,
tid')`. A transaction whose snapshot predates the retag must still find
`(7, EmailPass, tid)` through the index, and the old tuple in the
`EmailPass` heap is still visible to it.

## engine/retag-snapshot/two-entries

The index holds `(7, EmailPass, tid)` and `(7, Google, tid')`. Each
probe filters by visibility in the heap named. The uniqueness check
skips the old entry because its tuple's deleter is the retagging
transaction, as Postgres skips dead versions. Vacuum removes the old
entry in the per-sum batched pass.

## engine/retag-snapshot/entry-history

One entry per key carries `(Google, tid')` and `(EmailPass, tid)`
until no snapshot needs the old one; the uniqueness check never sees
two entries. Index entries become variable length.

## engine/retag-snapshot/live

Accept the in-place update.
