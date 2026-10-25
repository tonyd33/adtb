---
title: How a nullable sum-typed column is matched
pick: one
raised-by: [sum/value]
options:
  null-ctor:
    title: No NULL, optionality is a constructor of the sum
  null-arm:
    title: Every match over a nullable sum gets an implicit NULL arm
  null-explicit:
    title: Match over a nullable sum requires an explicit NULL arm
---
A column of type `Address` may be NULL in SQL. A match must say what a
NULL row produces.

## query/optional-sum/null-ctor

`address: Address` with `type Address = None | US { ... } | Intl { ... }`;
no field is nullable.

## query/optional-sum/null-arm

NULL in, NULL out, as SQL functions on NULL do.

## query/optional-sum/null-explicit

```
match a | null -> 'none' | US u -> u.zip | Intl i -> i.postcode
```
