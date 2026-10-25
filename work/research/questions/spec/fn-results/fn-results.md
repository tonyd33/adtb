---
title: "Whether a `fn` may return a function"
pick: one
options:
  allowed:
    title: "Yes; function types never appear in a relation, field, set element, type argument, query result or registered-query parameter; all normalised away before planning"
  parameters-only:
    title: "Function types only as parameters"
---
Raised by the third review of the spec in docs/spec/.

## spec/fn-results/allowed

Yes; function types never appear in a relation, field, set element, type argument, query result or registered-query parameter; all normalised away before planning. Higher-order helpers compose.

## spec/fn-results/parameters-only

Function types only as parameters. Rules out curried helpers.
