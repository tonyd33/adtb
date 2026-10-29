---
title: A set-valued join point is reached under a disjunction of conditions
pick: one
raised-by: [lang/patterns/failure-sharing/join]
options:
  live:
    title: Plan and delta each jump as its own branch
  disjunction:
    title: Plan the join body once under the disjunction of the jump conditions
---
In a query, `F` is a set reached on every path that fails. The planner
lowers an `If` chain to filtered union branches and the incremental check
records the guards of each read; with a join point, `F`'s branch filter
is the disjunction of the conditions at the jumps.

## lang/patterns/failure-sharing/join-planning/live

The planner expands each jump to a copy of `F`: the plan is exponential
again, only the kernel term is linear.

## lang/patterns/failure-sharing/join-planning/disjunction

The planner and `write/delta` collect the path condition at each jump
and plan `F` once under their disjunction. New machinery in both.
