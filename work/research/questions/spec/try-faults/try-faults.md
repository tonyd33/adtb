---
title: "What `try` reports when several bindings fault"
pick: one
options:
  any:
    title: "Any one of the faults; a listed freedom; conformance accepts any fault some binding raises"
  least:
    title: "The fault of the least binding in value order"
---
Raised by the third review of the spec in docs/spec/.

## spec/try-faults/any

Any one of the faults; a listed freedom; conformance accepts any fault some binding raises. Keeps parallel evaluation free.

## spec/try-faults/least

The fault of the least binding in value order. Forces full evaluation and an order on every binding.
