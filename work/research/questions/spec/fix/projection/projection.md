---
title: "Whether a primitive returning a component of an argument is constructing"
pick: one
options:
  not-constructing:
    title: "Not constructing: start, end, unwrap and value return a component of an argument"
  constructing:
    title: "Constructing, since the result is none of the arguments"
---
Raised by the boundedness analysis. semantics.md §5.10 called
constructing any primitive whose result may be none of its arguments,
which takes in projections such as `unwrap(n)` or `start(s)`.

## spec/fix/projection/not-constructing

A primitive whose result is a component of an argument (`start`,
`end`, `unwrap`, `value`) is not constructing. Its results are drawn
from finitely many values present before the `fix` runs.

## spec/fix/projection/constructing

A reachability `fix` over newtype keys that unwraps them is unbounded,
though it builds no new value.
