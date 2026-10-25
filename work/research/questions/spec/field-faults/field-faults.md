---
title: "Which fault is reported among the fields of one value"
pick: one
options:
  any:
    title: "Any fault some field raises; the fields of one record or constructor value are not ordered"
  as-written:
    title: "The first fault in the order the fields are written"
  by-name:
    title: "The first fault in the code-point order of the field names"
---
Raised by the reference interpreter. semantics.md §5.1 sequenced the
fields of a record and a constructor value left to right as written,
while the kernel sorts fields by name. In

```adt fragment
{ b: 1 / z, a: x + 1 }
```

with `z == 0` and `x` the greatest `I64`, the reported fault is
`DivisionByZero` in written order and `ArithmeticOverflow` in name
order.

## spec/field-faults/any

Every field is evaluated before the value is built, and when more than
one faults the outcome is the fault of any one of them. A freedom like
the one among several bindings (semantics.md §1.1), and listed with it
in the conformance table. Whether the value faults is still fixed;
only which fault is free. Operands of an operator and arguments of a
call stay left to right.

## spec/field-faults/as-written

The written order decides. The kernel has to keep each value's
written field order beside its sorted type, and two values of one
type written in different orders get different fault outcomes.

## spec/field-faults/by-name

The sorted order decides. Deterministic, but the fault reported
contradicts the text: `{ b: 1 / z, a: x + 1 }` reports the overflow
although the division is written first.
