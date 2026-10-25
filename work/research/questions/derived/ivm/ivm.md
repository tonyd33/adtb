---
title: How a stored derived relation is maintained
pick: some
raised-by: [derived/freshness/in-txn, derived/freshness/lagging-snapshot, derived/freshness/eventual, derived/freshness/on-refresh]
options:
  recompute:
    title: Rerun the query and replace the stored relation
    assessment: {grade: costly, why: cost is the view's input size per maintenance}
    links:
      - {rel: prior-art, to: prior/derived-pg-matview}
  delta-rules:
    title: Compile per-relation delta queries over the change set and base indexes
    links:
      - {rel: same-as, to: constraint/enforce/delta}
      - {rel: prior-art, to: prior/derived-gupta-mumick}
      - {rel: prior-art, to: prior/constraint-simplification}
  circuit:
    title: Compile the query into an incremental dataflow circuit with operator state
    links:
      - {rel: same-as, to: constraint/enforce/ivm}
      - {rel: prior-art, to: prior/derived-dbsp}
      - {rel: prior-art, to: prior/derived-dd}
links:
  - {rel: same-as, to: net/feed/maintain/incremental}
---
A constraint is a view that must stay empty, so whatever compiles
constraint deltas (`constraint/enforce/delta`) already maintains a
class of views. A `match` view lowers to a union of arm queries
(`query/union-all`); its delta is the union of arm deltas, and a write
to `identities.Google` drives only the arms reading that heap.

## derived/ivm/recompute

Correct for any query. The only method for `on-refresh`; Oracle
`COMPLETE`.

## derived/ivm/delta-rules

```
on change d to invoices.Open:
  for each user in d: open_by_user[user].open += |inserted| - |deleted|
```

Nicolas 1982 simplification extended to views: counting keeps a
derivation count per stored row so a set-valued view
(`lang/collection/sets`) can drop a row only when its last derivation
goes; DRed handles `lang/recursion/fix` views by deleting a superset
and rederiving. Gupta, Mumick, Subrahmanian, SIGMOD 1993. Oracle `FAST`
refresh needs `COUNT(*)` beside each aggregate for the same reason.

## derived/ivm/circuit

DBSP: `Q^Δ = D ∘ ↑Q ∘ I`, linear operators are their own increment,
join is bilinear, distinct is incremental at change size. Handles
nested results (`lang/results/nested`), aggregation, recursion and
stratified negation uniformly; MIN needs the full group or a custom
operator (`kb/dbsp.md`). Differential Dataflow adds partially ordered
versions for iteration.
