---
title: Where incremental maintenance keeps its intermediate state
pick: some
raised-by: [derived/ivm/delta-rules, derived/ivm/circuit]
options:
  base-indexes:
    title: Delta queries probe base relations through their indexes; no private state
    links:
      - {rel: same-as, to: constraint/admit/bounded}
  arrangements:
    title: Stateful operators keep private indexed copies of their inputs
    links:
      - {rel: prior-art, to: prior/derived-dd}
      - {rel: prior-art, to: prior/derived-dbsp}
  partial:
    title: State is kept only for keys that have been read and rebuilt on a miss
    links:
      - {rel: prior-art, to: prior/derived-noria}
---
DBSP's incremental join and distinct hold O(|DB|) state. A database
already holds its base relations, indexed and versioned.

## derived/ivm/state/base-indexes

The delta of `users ⋈ invoices` for an inserted invoice probes
`users` by key at the transaction's snapshot. State is the stored view
plus derivation counts. Admits only plans whose every join from a
delta has an index, the class `constraint/admit/bounded` admits.

## derived/ivm/state/arrangements

Differential Dataflow arrangements, DBSP integrals: each join input
or distinct keeps its own indexed trace. Any plan is incremental;
memory grows with every stored view.

## derived/ivm/state/partial

```
fn invoices_of (u: Key users) -> Set Invoice = ...
store invoices_of partial
```

Only parameters that have been read are stored; a miss evaluates the
query for that key (Noria upqueries); eviction is allowed. Fits
registered parameterized queries (`lang/params/declared`). Noria is
eventually consistent; under MVCC a miss can evaluate at the reader's
snapshot (my inference, unverified).
