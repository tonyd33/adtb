---
title: Float aggregates depend on evaluation order
pick: any
raised-by: [project/oracle/rust-reference, project/oracle/prover-model]
options:
  tolerance:
    title: Compare float results within a bound derived from the input size
  fixed-order:
    title: The spec fixes the summation order of float aggregates
  exact-reference:
    title: The oracle computes exactly; the engine must lie within a stated error
  live:
    title: Exclude floats from differential tests
    excludes: [project/oracle/float/tolerance, project/oracle/float/fixed-order, project/oracle/float/exact-reference]
---
`model/float` is in the base set. Morsel-parallel `sum` over a float
field adds in a different order on every run; the oracle adds in
another. Sets are compared as sets, so order of rows is not an issue
(`lang/order/result-only`); order of addition is.

```
from e in events group e by e.kind into g
select { kind: g.key, total: sum(x.amount for x in g) }
```

## project/oracle/float/tolerance

Results equal if each float differs by at most n·ε·Σ|x|.

## project/oracle/float/fixed-order

The spec says `sum` over floats adds in key order; parallel plans must
reproduce it (tree reduction over key ranges). Deterministic results;
a cost on every parallel aggregate.

## project/oracle/float/exact-reference

The oracle sums as rationals; the spec states the engine's bound.

## project/oracle/float/live

Differential tests generate only exact types.
