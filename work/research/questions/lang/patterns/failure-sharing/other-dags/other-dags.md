---
title: Other sharing in kernel terms still meets tree walkers
pick: any
raised-by: [lang/patterns/failure-sharing/join, lang/patterns/failure-sharing/fold, lang/patterns/failure-sharing/retest]
options:
  live:
    title: Leave the tree walkers until another source of sharing is measured
    excludes: [lang/patterns/failure-sharing/other-dags/context-free]
  context-free:
    title: Memoise the context-free tree walkers on term identity
---
Terms stay DAGs (docs/design/kernel.md §2): the normaliser shares an
inlined body's result between call sites, and or-pattern alternatives
share the arm body. `Terms::any` (kernel.rs:346),
`write_depends_on_volatile` (analysis/volatile.rs:23) and
`reads_only_row` (check/mod.rs:825) walk as trees and would be
exponential on nested sharing from any source.

## lang/patterns/failure-sharing/other-dags/live

No other source is known to compound.

## lang/patterns/failure-sharing/other-dags/context-free

A visited set in `Terms::any`, and a memo in the walkers whose result
depends on the node alone. Hygiene, independent of the match lowering.
