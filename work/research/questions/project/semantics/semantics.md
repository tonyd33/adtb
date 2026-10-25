---
title: How the spec defines what queries and writes mean
pick: one
options:
  prose:
    title: Precise prose, as the SQL standard
    links:
      - {rel: prior-art, to: prior/project-sql-standard-structure}
  denotational:
    title: A denotational semantics on paper, over sets as NRC
    links:
      - {rel: prior-art, to: prior/lang-nrc}
      - {rel: prior-art, to: prior/project-formal-sql}
  mechanised:
    title: A denotational semantics mechanised in a proof assistant
    links:
      - {rel: prior-art, to: prior/project-formal-sql}
      - {rel: prior-art, to: prior/project-cedar}
---
The language is new and has no other implementation to agree with, so
whatever the spec says is the only definition. Settled rulings already
give it a set semantics: relations are sets (`model/sets`), a sum is
the disjoint union of its constructor relations (`query/union-all`),
`match` lowers to a union of arm queries, a write denotes inserted and
deleted relations per constructor (`write/primitive/delta`), a
constraint is a query that must return nothing (`constraint/form/empty`).

## project/semantics/prose

Natural language, as ISO/IEC 9075. Guagliardo and Libkin found that
"written in natural language, which is inherently ambiguous" is why
vendors read SQL differently (PVLDB 11(1), 2017).

## project/semantics/denotational

Each construct gets a meaning as a function of a database state, in
the style of NRC comprehensions:

```
[[ from x in r where p select e ]] D = { [[e]](x:=v) | v ∈ [[r]] D, [[p]](x:=v) }
[[ identities ]] D = ⊎_{C ∈ ctors(Identity)} [[ identities.C ]] D
[[ match i | EmailPass e -> a | Google g -> b ]] = [[a]] on EmailPass rows ∪ [[b]] on Google rows
[[ update ... ]] D = (Δ⁺_C, Δ⁻_C) for each constructor C
```

Guagliardo and Libkin wrote such a semantics for basic SQL and checked
it by running it beside PostgreSQL and Oracle.

## project/semantics/mechanised

The same definitions in Lean, Rocq or Isabelle, with proofs of the
laws the planner relies on (arm distribution, pruning by constructor
type, normalisation of higher-order abstraction). HoTTSQL proved
optimizer rewrites this way; SQLCoq proved SQL equivalent to a bag
algebra; Cedar's Lean models are its specification.

```
theorem prune_ctor (q : Query) (h : q.reads ⊆ {Identity.Google}) :
  eval q D = eval q (D.restrict {Identity.Google})
```
