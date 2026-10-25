---
title: Which base scalar types the engine provides
pick: some
options:
  int:
    title: Fixed-width signed integers
  bigint-exact:
    title: Arbitrary-precision integers
  decimal:
    title: Exact decimal with declared scale
  float:
    title: IEEE 754 binary floats
  text:
    title: UTF-8 text
  bytes:
    title: Byte strings
  time:
    title: Instants and civil dates and times as distinct types
  prelude-bool:
    title: Bool is a prelude sum False or True
    needs: [model/nullary-sum]
---
What every user type bottoms out in. Booleans and optionality need no
primitive once nullary sums exist.

## model/int

`I32`, `I64`.

## model/bigint-exact

`Int` unbounded; no overflow semantics to specify.

## model/decimal

`Decimal 2` for money.

## model/float

`F64`; equality and ordering of NaN must be fixed for keys and indexes.

## model/text

`Text`, comparison by code point unless a collation is named.

## model/bytes

`Bytes`.

## model/time

`Instant`, `Date`, `Time`, `DateTime` (civil, no zone), `Duration`, as
in java.time
(https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/time/package-summary.html).

## model/prelude-bool

`type Bool = False | True`; `where` takes a `Bool`; `match` on it is
`if`.
