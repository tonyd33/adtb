---
title: Inferred lattice types are hard to read in errors
pick: any
raised-by: [types/infer/mlsub]
options:
  simplify:
    title: Simplify types by automaton minimisation before showing them
    links:
      - {rel: prior-art, to: prior/types-mlsub}
  sig-only:
    title: Report errors only against written signatures, never inferred schemes
  live:
    title: Live with lattice expressions in errors
    excludes: [types/infer/mlsub-display/simplify, types/infer/mlsub-display/sig-only]
---
A failed biunification reports a constraint like
`Identity.Google ⊔ α <= Key users` whose variable came from another
arm or binder.

## types/infer/mlsub-display/simplify

MLsub: two automata denote equal types iff they accept equal languages,
so any minimisation algorithm applies.

## types/infer/mlsub-display/sig-only

Since every registered query and function has a signature, an error
names the signature it violates and the arm that violated it.

## types/infer/mlsub-display/live

Show the constraint as solved.
