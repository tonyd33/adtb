---
title: Which proof assistant holds the semantics
pick: one
raised-by: [project/semantics/mechanised]
options:
  lean:
    title: Lean 4
    links:
      - {rel: prior-art, to: prior/project-cedar}
  rocq:
    title: Rocq (Coq)
    links:
      - {rel: prior-art, to: prior/project-formal-sql}
  isabelle:
    title: Isabelle/HOL
---
The choice decides whether the model doubles as a fast executable
oracle and who can read it.

## project/semantics/prover/lean

Lean 4 is also a compiled programming language; Cedar runs its Lean
models as test oracles against the Rust implementation.

```
def evalFrom (r : Rel α) (p : α → Bool) (e : α → β) : Finset β :=
  (r.filter p).image e
```

## project/semantics/prover/rocq

HoTTSQL and SQLCoq precedent; extraction to OCaml gives an executable
semantics (SQLCoq ships a certified semantic analyser this way).

## project/semantics/prover/isabelle

Isabelle/HOL with code generation to Haskell, OCaml or Scala
(unverified as a database precedent here). Strong automation
(Sledgehammer) for set-theoretic goals.
