---
title: Splitting or merging constructors rewrites heaps under SIREAD locks
pick: any
raised-by: [[engine/ssi, evolution/split], [engine/ssi, evolution/merge]]
options:
  promote-heaps:
    title: Tuple and page locks on the rewritten heaps become relation locks on every heap the operator writes
    links:
      - {rel: prior-art, to: prior/mvcc-pg-ssi-internals}
  live:
    title: Live with locks naming TIDs that no longer exist
    excludes: [mvcc/split-locks/promote-heaps]
    assessment: {grade: broken, why: a serializable reader's locks miss writes to the moved rows}
---
SIREAD locks outlive their transaction and do not block DDL. A split of
`identities.Social` into `Google | Facebook` moves rows to new heaps with
new TIDs. Postgres promotes a rewritten table's tuple and page locks to
relation locks and turns a dropped index's gap locks into a heap lock
(Ports and Grittner sect. 5.2).

## mvcc/split-locks/promote-heaps

Locks on `identities.Social` and its index ranges are replaced by one
relation lock on every heap the operator writes, so any later write to
those constructors conflicts with the old readers.

## mvcc/split-locks/live

No promotion.
