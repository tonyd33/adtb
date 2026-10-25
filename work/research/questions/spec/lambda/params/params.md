---
title: "The type of a function value with several parameters"
pick: one
options:
  curried:
    title: "fn (a: T, b: U) -> e has type T -> U -> R; a call applies its arguments in order"
  one-param:
    title: "A lambda takes one parameter"
  record-arg:
    title: "Several parameters are one record argument"
---
Raised while implementing the front end against docs/spec/. The lambda
grammar admits a parameter list and function types are `T -> U`, but
nothing gave a lambda or a named schema `fn` with several parameters a
type, or said what `f(x, y)` means for a function value.

## spec/lambda/params/curried

`fn (a: T, b: U) -> e` has type `T -> U -> R`, and a call `f(x, y)` of
a function value applies its arguments in order. A schema `fn` with
parameters `T, U` named without a call has that curried type; a
parameterless one has type `() -> R`.

```adt
fn having2 (p: Identity -> Key users -> Bool, u: Key users) -> Set (Key identities) =
  from i in identities where p(i, u) select key i
```

## spec/lambda/params/one-param

A lambda takes one parameter and a function type one argument; several
arguments are nested lambdas. A named `fn` with several parameters
cannot be passed.

## spec/lambda/params/record-arg

Parameters form one anonymous record argument, `{ a: T, b: U } -> R`.
Puts function arguments under record equality and projection rules.
