---
title: A reference valid over a period needs its target to cover that period
pick: any
raised-by: [derived/valid-time/pattern, derived/valid-time/period]
options:
  covered:
    title: The engine checks the child's period lies in the union of target periods
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  single-row:
    title: The child's period must lie inside one target row's period
  live:
    title: Live with temporal references checked only by key
    excludes: [derived/valid-time/temporal-fk/covered, derived/valid-time/temporal-fk/single-row]
---
An assignment of employee 3 to department 7 for 2025 needs department
7 to exist throughout 2025, possibly across several rows of its
history.

## derived/valid-time/temporal-fk/covered

SQL:2011 temporal FK semantics. The check unions adjacent target
periods; a delete or shortening of one target row rechecks children.
An aggregate-like check that `constraint/admit/bounded` would refuse
unless built in.

## derived/valid-time/temporal-fk/single-row

An inclusion constraint with a range probe, index-bounded; forces
targets to be stored unsplit.

## derived/valid-time/temporal-fk/live

`Key departments` only; coverage is the application's job.
