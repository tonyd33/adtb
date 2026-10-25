---
title: The type of a numeric literal
pick: one
raised-by: [model/int]
options:
  fixed:
    title: Each literal form has one type
  polymorphic:
    title: A literal takes the numeric type its context expects, with a default
  suffixed:
    title: Unsuffixed literals have one type; suffixes name the rest
---
`where o.qty > 3` with `qty: I32`, `price * 1.1` with `price: Decimal 2`.

## types/literals/fixed

`3 : I64`, `3.0 : F64`; `o.qty > 3` is a type error with `qty: I32`
unless conversions are implicit (`types/num-mix`).

## types/literals/polymorphic

`3` checks against any integer type and `1.1` against any decimal or
float type; out-of-range literals are compile errors; with no context
`3 : I64`. Haskell `fromInteger` with defaulting, Rust integer literal
inference with `i32` fallback.

## types/literals/suffixed

`3`, `3i32`, `1.10d`, `1.1f`. Combines with `polymorphic` in Rust; here
the unsuffixed form is fixed.
