---
title: What == and order mean on floats
pick: one
raised-by: [model/float]
options:
  nan-greatest:
    title: All NaNs equal and above every number; -0.0 equals 0.0
    links:
      - {rel: prior-art, to: prior/types-float}
  no-nan:
    title: NaN is not a float value; an operation producing it faults
  ieee:
    title: IEEE comparison, with NaN unequal to itself
    assessment: {grade: broken, why: "== is not reflexive, so set deduplication and unique keys have no equivalence to use"}
links:
  - {rel: variant-of, to: btree/keyenc/canon/canonicalize}
---
Floats sit in keys, sets and sums, and the index already encodes
`-0.0` as `0.0` and every NaN as one NaN
(`btree/keyenc/canon/canonicalize`). The language's `==` must be the
equivalence that encoding implements.

## types/float-eq/nan-greatest

Postgres float and numeric: NaNs equal each other and sort above every
non-NaN. OCaml `compare` with NaN least is the mirror image.

```
from m in metrics where m.v > 100.0 select m    -- NaN rows included
```

## types/float-eq/no-nan

`F64` holds the real numbers and the two infinities; `0.0 / 0.0` and
`inf - inf` fault (`types/fault`). The order is total with no special
case. Rust's `ordered_float::NotNan` (https://docs.rs/ordered-float, unverified).

## types/float-eq/ieee

Rust `PartialEq` for `f64`; the index then disagrees with `==`.
