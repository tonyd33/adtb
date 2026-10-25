---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 200
summary: "adtb-reference has no evaluator for queries, and the kernel term it would read has no written contract"
branch: interp
tags: [reference]
---

# Feature: reference interpreter for queries

## Report

`adtb-reference` cannot evaluate a query. Milestone 2 of `docs/plan.md`
needs a naive evaluator of the semantics equations
(`docs/spec/semantics.md` §4 and §5) over
`BTreeMap<(RelVarId, CtorId), BTreeSet<Row>>`, reading the kernel terms
`adtb-lang` produces: comprehensions, `match` over constructor
relations, `fix`, grouping, aggregates, ordering, top-n, faults and
`try`.

The kernel term has no contract the interpreter can be written against.
`docs/design/kernel.md` must state the forms, their invariants and the
meaning each has against the semantics equations, as sections 1 and 2.
Writes and blocks are section 3 (`kernel-writes-contract.md`).

## 2026-10-25: queries done

The query evaluator is done in workspace `interp` and awaits landing.
Its findings are filed as `front-end-decimal-literals.md`,
`front-end-ctor-comparison-join.md`, `agg-source-nonempty.md`,
`anon-sum-ctor-index.md`, `spec-field-eval-order.md`,
`spec-take-negative.md`, `spec-textual-specials.md` and
`reference-unicode-text.md`.
