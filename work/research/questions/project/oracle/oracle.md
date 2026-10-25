---
title: An executable reference semantics as a test oracle
pick: one
options:
  none:
    title: No reference implementation; tests state expected results by hand
  rust-reference:
    title: A naive interpreter in Rust over in-memory sets, in the repository
    links:
      - {rel: prior-art, to: prior/project-wasm-spec}
      - {rel: prior-art, to: prior/project-formal-sql}
  prover-model:
    title: The mechanised model, run as the oracle
    needs-one-of: [project/semantics/prover/lean, project/semantics/prover/rocq, project/semantics/prover/isabelle]
    links:
      - {rel: prior-art, to: prior/project-cedar}
---
The engine plans, prunes constructors, distributes joins into arms,
fuses probes of the union index and compiles hot queries. Every one of
those is a rewrite that must preserve meaning. A reference that
evaluates the semantics literally (scan every constructor relation,
nested loops, materialise everything) answers what the result should
be for any generated query and database.

## project/oracle/none

Expected outputs are written by hand or recorded from the engine
(golden files). Recording catches regressions, not wrong answers.

## project/oracle/rust-reference

The WebAssembly reference interpreter is written "for clarity and
simplicity, not speed"; Guagliardo and Libkin's Python semantics capped
tables at 50 rows. Here it is a crate sharing nothing with the engine
but the typed AST.

```
fn eval(q: &Kernel, db: &BTreeMap<CtorId, BTreeSet<Row>>) -> BTreeSet<Value>
// identities = ⋃ over ctors; match = union of arms; no indexes, no plans
```

## project/oracle/prover-model

Cedar: Lean models are the specification and the oracle; the Rust
implementation is checked against them by differential random testing.
Proof and oracle cannot drift apart.
