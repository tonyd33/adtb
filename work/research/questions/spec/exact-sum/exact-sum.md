---
title: "How `sum` over integers and decimals overflows"
pick: one
options:
  exact:
    title: "The exact total, checked once at the end"
  per-step:
    title: "`+` faulting at each step"
---
Raised by the third review of the spec in docs/spec/.

## spec/exact-sum/exact

The exact total, checked once at the end. Overflow does not depend on fold order.

## spec/exact-sum/per-step

`+` faulting at each step. Whether it faults depends on order.
