---
title: Arithmetic and comparison across numeric types
pick: one
raised-by: [model/int]
options:
  explicit:
    title: Operands must have one type; conversions are written
  widen:
    title: Implicit lossless widening to a common type
  subtype:
    title: Narrower numeric types are subtypes of wider ones
---
`i32_col + i64_col`, `int_col * decimal_col`, `int_col < float_col`.

## types/num-mix/explicit

```
from o in orders select i64(o.qty) * o.unit_cents
```

Rust. Comparison between `I64` and `F64` has no lossless common type,
so it is always written.

## types/num-mix/widen

I32 to I64, integers to `Decimal`, never to float implicitly. SQL
numeric promotion (SQL:2016 sec. 6.29, unverified).

## types/num-mix/subtype

`I32 <: I64` in the same order as constructor sets, so joins of match
arms widen numbers too. A subtyping coercion changes representation,
unlike constructor-set subsumption.
