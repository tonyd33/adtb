---
title: What fixed-width integer overflow does
pick: one
raised-by: [model/int]
options:
  trap:
    title: Overflow is a fault
    links:
      - {rel: prior-art, to: prior/types-numeric}
  wrap:
    title: Arithmetic is modular
  saturate:
    title: Results clamp to the type's range
  widen:
    title: Each operation's result type is wide enough for its operands
---
`I64` sum of a column, `I32` product of two fields.

## types/int-overflow/trap

Postgres integer types; Rust's intended semantics (RFC 560: overflow is
a program error). Follows `types/fault`.

## types/int-overflow/wrap

OCaml `int`, Java, Pony's default operators. A wrong total is a silent
result.

## types/int-overflow/saturate

Rust `saturating_*` as the default operator.

## types/int-overflow/widen

`I32 * I32 : I64`, as SQL Server grows decimal precision per operator.
Stops at the widest type, where it needs one of the others; `sum` over
a relation has no static bound.
