---
title: Evolving a parametric sum changes every instantiation
pick: any
raised-by: [model/user-params]
options:
  per-instance:
    title: Operators target one instantiation and fork it into a new type
  frozen:
    title: A parametric sum cannot evolve once a relation uses it
  live:
    title: Live with one operator reaching every instantiation
    excludes: [model/param-evolution/per-instance, model/param-evolution/frozen]
---
`evolution/add` of a constructor to `Result a e` reaches every relation
using any `Result`, and every exhaustive match on one breaks.

## model/param-evolution/per-instance

Adding `Retry` to the `Result Job Failure` in `jobs` gives it a new
nominal type; other uses keep `Result`.

## model/param-evolution/frozen

Prelude-like types are fixed; a variant needs a new declaration.

## model/param-evolution/live

One operator, rechecked against every dependent query
(`evolution/breaks/recompile`).
