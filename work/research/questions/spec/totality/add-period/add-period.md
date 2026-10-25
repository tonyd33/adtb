---
title: "Whether add_period with Reject is total"
pick: one
options:
  partial:
    title: "Partial: it can fault with ArithmeticOverflow"
  total:
    title: "Total, since it returns Option"
---
Raised by the totality analysis. typing.md §10.1 called
`add_period(..., Reject)` total because it returns `Option`, yet
prelude.md §6.3 says every `add_period` faults with
`ArithmeticOverflow` when the result leaves the type's range.

## spec/totality/add-period/partial

`add_period` is partial with either rule. `Reject` returns `None` only
for a day the month does not have.

## spec/totality/add-period/total

`Reject` also returns `None` on overflow. Changes prelude.md §6.3 and
merges two different failures into one `None`.
