---
title: Guards make coverage undecidable
pick: any
raised-by: [lang/patterns/guards]
options:
  ignore-guarded:
    title: Guarded arms count for nothing toward coverage
  live:
    title: Live with guards trusted to cover
    excludes: [lang/patterns/guard-cover/ignore-guarded]
---
A guard is an arbitrary predicate. `| Google g if p | Google g if not p`
covers `Google` only if the checker proves `p or not p`.

## lang/patterns/guard-cover/ignore-guarded

Rust and OCaml: a guarded arm is assumed to fail, so an unguarded arm
for the constructor is still required. GHC reasons about guards more
precisely (Graf, Peyton Jones, Scott, "Lower your guards", ICFP 2020).

## lang/patterns/guard-cover/live

A run-time error when no arm matches, breaking `query/compile-time`.
