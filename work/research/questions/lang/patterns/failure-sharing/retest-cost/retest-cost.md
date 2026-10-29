---
title: The body repeats the pattern's tests at run time
pick: any
raised-by: [lang/patterns/failure-sharing/retest]
options:
  live:
    title: Live with one repeated total test per nested constructor
    excludes: [lang/patterns/failure-sharing/retest-cost/share]
  share:
    title: The planner shares the test between condition and body
---
For a nested constructor pattern, the condition's `Case` tests the field
and the body's `Case` tests it again to bind the row. Literal tests and
guards are not repeated; only constructor destructuring is.

## lang/patterns/failure-sharing/retest-cost/live

A tag read and a branch per nested constructor in the matching arm only;
the failing arms run as today. The lowering already retests across arms:
each arm tests the scrutinee from scratch.

## lang/patterns/failure-sharing/retest-cost/share

The executor recognises the body's `Case` on a term the condition just
tested and reuses the outcome. An optimisation, not a semantic change.
