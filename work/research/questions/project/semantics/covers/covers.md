---
title: What the formal semantics covers
pick: some
raised-by: [project/semantics/denotational, project/semantics/mechanised]
options:
  queries:
    title: Queries, including match, grouping, fixpoints and nested results
  writes:
    title: Change sets and their application as one multiple assignment
  constraints:
    title: Constraints as queries that must return nothing on the new state
  isolation:
    title: Serializability as acyclic dependency graphs over constructor relations
    links:
      - {rel: prior-art, to: prior/project-elle}
  evolution:
    title: Schema operators, their inverses and version translation
    links:
      - {rel: prior-art, to: prior/prism-bidel}
---
Each layer named here gets equations and becomes checkable by the
oracle; anything left out is defined by prose and tested only by
examples.

## project/semantics/covers/queries

`[[q]] : DB → Rel`. The base.

## project/semantics/covers/writes

`[[w]] : DB → (Δ⁺_C, Δ⁻_C)_C` and `apply : DB × Δ → DB`, with retag as
a deletion from one constructor and an insertion into another under one
key (`write/retag/assign`).

## project/semantics/covers/constraints

```
valid(D) ⇔ ∀ c ∈ constraints. [[c]] D = ∅
commit(D, Δ) = apply(D, Δ)  if valid(apply(D, Δ)), else refuse with [[c]] (apply(D, Δ))
```

## project/semantics/covers/isolation

Adya's phenomena with reads and writes on constructor relations; a
read of `identities` is a predicate read over every heap
(`engine/guarantees-only`). The spec's promise is that every committed
history has an acyclic serialization graph.

## project/semantics/covers/evolution

Each operator is a pair of maps between database states of two schema
versions, with the inverse law `inv(op)(op(D)) = D` except where a loss
is declared (`evolution/smo`). Queries pinned to the old version are
translated and must agree.
