---
title: Negation and aggregation through recursion
pick: some
raised-by: [lang/recursion/rules, lang/recursion/fix]
options:
  positive:
    title: Recursive relations used only positively
    excludes: [lang/recursion/negation/stratified, lang/recursion/negation/lattice]
  stratified:
    title: Negation and aggregation only of lower strata
    links:
      - {rel: prior-art, to: prior/lang-datalog}
  lattice:
    title: Monotone aggregation inside recursion over declared lattices
    links:
      - {rel: prior-art, to: prior/flix}
---
`not exists` or `count` over a relation being defined has no unique
fixpoint. A `match` arm does not negate: arms range over disjoint
constructor relations, so matching inside a rule body stays positive.

## lang/recursion/negation/positive

No `not exists` and no aggregate over a recursive relation in its own
definition, nor anywhere it is still being computed.

## lang/recursion/negation/stratified

```
from n in nodes where not exists r in reach where r == key n select n   -- reach in a lower stratum
```

A program with a cycle through negation is rejected at compile time,
as Soufflé and Flix do.

## lang/recursion/negation/lattice

```
lat dist(n, d: Min Int)
dist(src, 0).
dist(m, d + w) :- dist(n, d), Edge(n, m, w).
```

Flix lattice semantics: shortest paths as a fixpoint, with the
aggregate monotone in a declared order.
