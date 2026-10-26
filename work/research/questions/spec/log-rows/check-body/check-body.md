---
title: "What the body Term of AddConstraint and AddCheck holds"
pick: one
raised-by: [spec/evolution/operator-type/catalog-sum]
options:
  whole-decl:
    title: "The whole declaration after the verb, so a check keeps its on target"
  predicate:
    title: "Only the predicate, the name and target dropped"
---
Raised while writing the migration log rows. `body: Term` did not say
how much of the declaration it holds.

## spec/log-rows/check-body/whole-decl

The `body` of `AddConstraint` and `AddCheck` is the declaration as
written after the verb, in canonical token form. A check keeps its
`on` target.

## spec/log-rows/check-body/predicate

Only the predicate. A check's target is then lost from the row, and
the hash does not cover it.
