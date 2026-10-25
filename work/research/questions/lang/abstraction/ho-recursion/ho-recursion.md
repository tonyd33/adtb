---
title: Higher-order normalisation does not cover recursion
pick: any
raised-by: [[lang/abstraction/higher-order, lang/recursion/rules], [lang/abstraction/higher-order, lang/recursion/fix]]
options:
  first-order-recursion:
    title: Recursive definitions take only first-order arguments
  live:
    title: Live with recursive higher-order queries the planner cannot flatten
    excludes: [lang/abstraction/ho-recursion/first-order-recursion]
---
The T-LINQ guarantee is for nonrecursive terms. A recursive function
taking a predicate cannot be inlined to a fixed query.

## lang/abstraction/ho-recursion/first-order-recursion

Lambdas are fine everywhere except as parameters of a `fix` or rule
definition; those are specialised by inlining at each call site first.

## lang/abstraction/ho-recursion/live

Such queries run with a closure per iteration, or fail to compile.
