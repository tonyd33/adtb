---
title: "Which types a query result may have"
pick: one
options:
  any:
    title: "Any type; a comprehension is a set"
  set-only:
    title: "Always Set T"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/query-result/any

Any type; a comprehension is a set. A single aggregate result needs no host comprehension.

## spec/query-result/set-only

Always Set T. Forced wrapping.
