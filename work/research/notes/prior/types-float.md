---
title: Float equality and order
---
IEEE: NaN unequal to itself, -0.0 == 0.0. Postgres treats NaNs as equal and greater than every non-NaN so floats can be indexed (https://www.postgresql.org/docs/current/datatype-numeric.html). Rust f64::total_cmp is IEEE totalOrder with -0.0 < +0.0 and NaNs at both ends (https://doc.rust-lang.org/std/primitive.f64.html). OCaml compare makes nan equal to itself and least.
