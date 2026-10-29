---
title: Path-sensitive passes cannot share a result across paths
pick: any
raised-by: [lang/patterns/failure-sharing/memo]
options:
  live:
    title: Live with exponential planning, code and deltas for multi-test matches
    excludes: [lang/patterns/failure-sharing/path-context/merge]
  merge:
    title: Each path-sensitive pass merges the contexts of the paths reaching a shared node
---
The delta of an incremental check records the guards along the path to
each read (crates/adtb/src/write/delta/mod.rs `top`, `nested`), the
planner tracks the constructors known on a path (plan/lower/mod.rs
`spine`), and the code generator emits one instruction block per path
(exec/code.rs `expr`). `F` is reached under `y == 1 and not g` on one
path and `y != 1` on the other, so a memo keyed on `Tm` and context
misses. Their output, not only their time, is exponential.

## lang/patterns/failure-sharing/path-context/live

Matches with many multi-test arms stay slow to plan and check.

## lang/patterns/failure-sharing/path-context/merge

Each pass finds the nodes with several parents, computes the disjunction
of the incoming contexts, and visits the node once under it; the code
generator emits shared blocks behind labels. This rebuilds join points
inside each pass without a kernel form naming them.
