---
title: Which integer widths exist
pick: some
raised-by: [model/int]
options:
  i64:
    title: I64
  sized:
    title: I8, I16, I32 beside I64
  unsigned:
    title: Unsigned U8 to U64
  i128:
    title: I128
---
Width is storage and a range check together; a range is also a domain
predicate (`constraint/domain`).

## types/int-widths/i64

One integer type. Narrower ranges are `type Port = new I64` with a
domain check.

## types/int-widths/sized

Postgres `smallint`, `integer`, `bigint`.

## types/int-widths/unsigned

Rust, ClickHouse; needs a rule for `U64` against `I64` in `types/num-mix`.

## types/int-widths/i128

Money in minor units of high-precision currencies, 128-bit ids as
numbers rather than `Bytes`.
