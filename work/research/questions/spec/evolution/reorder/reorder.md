---
title: "Whether an operator reorders the constructors of a sum"
pick: one
raised-by: [spec/evolution/ctor-order/placed]
options:
  operator:
    title: "A reorder operator, its inverse the old order"
  none:
    title: "No reorder; order is set by placement at add and split"
---
Raised by the implementability review of the spec in docs/spec/. The
spec said reordering constructors is a schema operator, but no
operator did it. Left open.

## spec/evolution/reorder/operator

`reorder constructors Stage as Draft | Approved | Sent | Paid`. Changes
the order of an ordered sum and so changes results of every sort and
range over it; a pinned reader would need the old order through
translation.

## spec/evolution/reorder/none

Order changes only by where `add constructor` and `split` place their
constructors. A wrong order is fixed by a new type.
