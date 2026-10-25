---
title: Values with one number and two scales are equal but distinguishable
pick: any
raised-by: [types/decimal/per-value]
options:
  normalize:
    title: A value is its number; trailing zeros are not kept
  unequal:
    title: 1.0 and 1.00 are different values
  live:
    title: Live with equal values of different scale
    excludes: [types/decimal/scale-eq/normalize, types/decimal/scale-eq/unequal, types/eq/leibniz]
---
With canonical key encoding (`btree/keyenc/canon/canonicalize`) an
index holds one encoding for `1.0` and `1.00`, so an index-only scan
returns a scale the heap row did not have. Postgres disables btree
deduplication for `numeric` for this reason.

## types/decimal/scale-eq/normalize

Scale becomes presentation, given by a format at output.

## types/decimal/scale-eq/unequal

`1.0 != 1.00`; sums and keys separate them.

## types/decimal/scale-eq/live

As Postgres.
