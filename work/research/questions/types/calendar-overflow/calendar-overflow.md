---
title: What adding months to a day that does not exist gives
pick: one
raised-by: [model/time]
options:
  clamp:
    title: Clamp to the last valid day
  fault:
    title: A fault
  explicit:
    title: The operation takes the overflow rule as an argument
---
`2021-01-31 + months(1)`.

## types/calendar-overflow/clamp

`2021-02-28`. Temporal's default `constrain`, java.time `plusMonths`,
Postgres.

## types/calendar-overflow/fault

Temporal `overflow: "reject"`.

## types/calendar-overflow/explicit

`add_months(d, 1, Clamp)` and `add_months(d, 1, Reject)`, the latter
returning `Option Date`.
