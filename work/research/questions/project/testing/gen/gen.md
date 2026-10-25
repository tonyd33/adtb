---
title: How random test inputs are generated
pick: some
raised-by: [project/testing/differential, project/testing/ctor-partition, project/testing/tlp, project/testing/plan-toggle, project/testing/pqs]
options:
  type-directed:
    title: Well-typed schemas, data and queries generated from the type system
  corpus-mutation:
    title: Mutate a corpus of queries and keep those that still type-check
  schema-walk:
    title: Random sequences of schema operators interleaved with writes and queries
---
Exhaustive `match` is a type check, so a random query is rejected
unless every match names every constructor (`lang/wildcard/scalars-only`).
Untyped generation spends almost all of its runs on type errors.

## project/testing/gen/type-directed

Generate a sum, its constructors and fields, rows per constructor
heap, then a query by type: a generator for each typing rule, choosing
`match` arms from the scrutinee's constructor set. QuickCheck and
Hypothesis style shrinking reduces a failure to a small schema and
query.

```
gen_query(Γ, τ) = choose [ from x in r ..., match e | C1 .. -> gen(Γ,τ) | C2 .. -> gen(Γ,τ), ... ]
```

## project/testing/gen/corpus-mutation

Start from the conformance suite and registered bundles; mutate
subterms. Reaches shapes users write; most mutants fail typing.

## project/testing/gen/schema-walk

Apply `add`, `split`, `merge`, `rename`, `move-field` at random to a
populated database, re-registering bundles each time, with the oracle
checking every query before and after. Exercises recompile, lazy page
rewrite (`layout/heap-format/page-version`) and the staged job
(`engine/ddl-online-txn/staged-job`).
