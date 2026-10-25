---
title: Whether functions take type parameters
pick: one
raised-by: [lang/inference/signatures]
options:
  monomorphic:
    title: Functions are monomorphic; only declared sums take parameters
  parametric:
    title: Functions take unconstrained type parameters
  bounded:
    title: Type parameters may require capabilities such as equality or order
---
`model/user-params` gives parametric sums. A helper over any
`Option a`, or a `max` over any ordered type, needs parametric
functions; ordering is per sum (`lang/sum-order/declared`), so `max`
over sums must ask for it.

## types/generics/monomorphic

```
fn or_dash (p: Option Text) -> Text = match p | None -> "-" | Some s -> s.value
```

## types/generics/parametric

```
fn or_else[a] (p: Option a, d: a) -> a = match p | None -> d | Some s -> s.value
```

ML polymorphism; normalisation (`lang/abstraction/higher-order`)
instantiates every call before planning.

## types/generics/bounded

```
fn top[a: Ord] (xs: Set a) -> Option a = ...
```

Haskell type classes, Rust trait bounds. The capabilities are fixed by
the engine (`Eq`, `Ord`, `Hash`, `Num`), not user-declared classes.
