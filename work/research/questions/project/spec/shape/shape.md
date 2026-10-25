---
title: What kind of document the spec is
pick: one
options:
  standard:
    title: Normative clauses with informative annexes, in numbered parts
    links:
      - {rel: prior-art, to: prior/project-sql-standard-structure}
  manual:
    title: The user documentation is the contract
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
  executable:
    title: Prose, a reference interpreter and an official test suite, released together
    links:
      - {rel: prior-art, to: prior/project-wasm-spec}
      - {rel: prior-art, to: prior/project-cedar}
  prescriptions:
    title: Numbered prescriptions and proscriptions, then the language that meets them
    links:
      - {rel: prior-art, to: prior/third-manifesto}
---
The project's goal is a spec for the data model and query language.
`engine/guarantees-only` settled that it states guarantees over
constructor relations and leaves heaps and pages to the
implementation.

## project/spec/shape/standard

ISO/IEC 9075: Framework, Foundation and further parts; normative text
says shall, annexes are informative. Conformance per feature.

## project/spec/shape/manual

SQLite and Postgres: the reference manual describes behaviour; the
SQLite file format document is a published contract
(https://www.sqlite.org/fileformat2.html). Normative and explanatory
text mix.

## project/spec/shape/executable

WebAssembly keeps the specification, a reference interpreter written
for clarity and the official test suite in one repository; Cedar's
Lean models are its specification. A change to the language changes
all three in one commit.

```
spec/
  model.md  lang.md  writes.md  txn.md  evolution.md   -- normative prose
  reference/   -- crate: kernel evaluator (project/oracle)
  suite/       -- conformance scripts
```

## project/spec/shape/prescriptions

The Third Manifesto lists RM Prescriptions and Proscriptions (no
nulls, no object ids, multiple assignment) and Tutorial D is one
language meeting them. Here: the model's commitments (closed sums,
disjoint totality, constraints as empty queries) stated apart from
the surface syntax.
