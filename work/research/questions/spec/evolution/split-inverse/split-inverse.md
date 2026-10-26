---
title: "When a split is invertible"
pick: one
options:
  every-product:
    title: "Invertible only when every product has an arm; a product no arm reaches makes the split lossy and its revert is refused naming it"
  binding-only:
    title: "Invertible when each arm binds or fixes every field and carries each binding whole"
---
Raised by `split Card into Visa | Other by | Card { last4 } -> Visa
{...}`, which meets the binding wording, but whose derived inverse
merge has no arm for `Other` and fails to type at revert.

## spec/evolution/split-inverse/every-product

A split is invertible only when its routing meets the binding
condition and every product is the result of some arm. A split with a
product no arm reaches is lossy, and its revert is refused, naming
that product.

## spec/evolution/split-inverse/binding-only

The binding condition alone. The inverse merge is then ill-typed for
a product no arm reaches, and the revert fails with a type error
rather than a refusal naming the cause.
