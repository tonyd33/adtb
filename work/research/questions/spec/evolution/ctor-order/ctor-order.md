---
title: "Where a new constructor sits in declaration order"
pick: one
options:
  placed:
    title: "add constructor appends last unless written before C or after C; split products take the source's position"
  append:
    title: "Always last"
---
Raised by the implementability review of the spec in docs/spec/.
Declaration order is the order of an ordered sum, and the spec did not
say where an added constructor or a split's products go.

## spec/evolution/ctor-order/placed

`add constructor` appends the constructor last unless written
`before C` or `after C`; split products take the source's position in
listed order:

```adt
-- type Stage ordered = Draft | Sent { at: Instant } | Paid { at: Instant, ref: Text }
add constructor Stage.Approved { at: Instant } after Sent
-- Draft < Sent < Approved < Paid
```

## spec/evolution/ctor-order/append

Every new constructor goes last. An ordered sum cannot gain a stage in
the middle; the author must merge and split, or add a new type.
