---
title: A stored -0.0 differs from its canonical index key
pick: any
raised-by: [types/float-eq/nan-greatest, types/float-eq/no-nan]
options:
  canon-store:
    title: Values are canonicalized on construction, so -0.0 never exists in the value domain
  heap-recheck:
    title: Float fields are never returned from the index; values come from the heap
    excludes: [types/eq/leibniz]
  live:
    title: Live with index-only scans returning 0.0 for a stored -0.0
    excludes: [types/float-eq/zero/canon-store, types/float-eq/zero/heap-recheck, types/eq/leibniz]
---
`1.0 / -0.0` is `-inf` and `1.0 / 0.0` is `inf`, so the two zeros are
`==` yet distinguishable, and an index-only scan returns the canonical
one.

## types/float-eq/zero/canon-store

Every float operation's result is canonicalized: `-0.0` becomes
`0.0` and NaN payloads collapse (under `nan-greatest`). `1.0 / -0.0`
cannot arise; `1.0 / 0.0` is `inf` or a fault. The heap, the index and
the wire carry one zero.

## types/float-eq/zero/heap-recheck

As `btree/keyenc/lossy/heap-recheck` for floats.

## types/float-eq/zero/live

Index-only and heap reads differ.
