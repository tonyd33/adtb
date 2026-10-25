---
title: "Which abort a block reports when several items abort"
pick: one
options:
  any-abort:
    title: "Any one of the aborts, as a declared freedom"
  written-order:
    title: "The first abort in written order"
---
Raised by the implementability review of docs/spec/. semantics §6.2
sequenced `∥` and `let!` left to right, so an implementation had to
evaluate a block's items in written order to report the right abort,
while writes §1.3 says the order of writes has no effect. The review
also found the components of `(w1, w2)` missing from writes §1.1's
list of write positions.

```adt
transact
  let a = n / 0   -- faults
  fail "no"
```

## spec/writes/abort-choice/any-abort

Which abort is reported when several `let!` bindings, writes or
`fail`s of one block would abort is a declared freedom, beside the one
semantics §1.1 states for bindings of a set. The components of
`(w1, w2)` are write positions.

## spec/writes/abort-choice/written-order

The first abort in written order. Forces sequential evaluation of a
block and makes the order of writes observable through errors.
