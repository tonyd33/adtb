---
title: "Aggregate syntax"
pick: one
options:
  comprehension:
    title: "agg(expr for x in q)"
  over:
    title: "agg expr over x in q"
---
Aggregates range over bindings (`lang/collection/set-agg/over-binders`), so the syntax must bind.

## syntax/aggregate/comprehension

`sum(l.amount for l in lines where l.entry == key e)`; the binder makes equal amounts count. Summing a projected set is a type error, which closes the set-semantics trap.

## syntax/aggregate/over

`sum l.amount over l in lines`.
