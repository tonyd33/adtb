---
title: How decimal precision and scale are typed
pick: one
raised-by: [model/decimal]
options:
  scale-typed:
    title: Scale is part of the type; result scales follow static rules and division names its scale
  sql-derived:
    title: Result precision and scale derived per operator and capped, rounding silently at the cap
    links:
      - {rel: prior-art, to: prior/types-numeric}
  per-value:
    title: Each value carries its own scale, as Postgres unconstrained numeric
---
`Decimal 2` for money is the base-types example. Whether `1.0` and
`1.00` are one value, and what `price * rate` and `total / n` return.

## types/decimal/scale-typed

```
price: Decimal 2, rate: Decimal 4
price * rate                       -- : Decimal 6
price + rate                       -- : Decimal 4, price rescaled exactly
div(total, n, scale: 2, round: HalfEven)   -- : Decimal 2
round(price * rate, 2, HalfEven)   -- : Decimal 2
```

Values are scaled integers of a fixed width (128-bit, 38 digits);
exceeding it is a fault. No rounding happens unless written.
`1.0 : Decimal 1` and `1.00 : Decimal 2` are different types; their
comparison rescales.

## types/decimal/sql-derived

SQL Server: `e1 * e2` has precision `p1 + p2 + 1`, scale `s1 + s2`;
`e1 / e2` scale `max(6, s1 + p2 + 1)`; above 38 the scale is cut,
rounding the fraction, down to 6 for multiplication and division.

## types/decimal/per-value

Postgres `numeric`: any precision; `1.0` and `1.00` compare equal but
display differently.
