---
title: What a user-declared scalar type is
pick: one
options:
  newtype:
    title: A distinct nominal type over a base type with explicit conversion
    links:
      - {rel: prior-art, to: prior/model-rel}
  subtype:
    title: A subset of a base type usable wherever the base type is
    links:
      - {rel: prior-art, to: prior/souffle}
      - {rel: prior-art, to: prior/third-manifesto}
  alias:
    title: Another name for the base type
links:
  - {rel: variant-of, to: constraint/domain}
---
`Email` over `Text`, `UserId` over `Int`, `Cents` over `Int`.

## model/newtype

```
type Email = new Text
type User = { email: Email }
insert users { email: "a@b" }         -- type error
insert users { email: Email "a@b" }   -- ok; text(u.email) gives Text back
```

A predicate on the type is `constraint/domain`. Haskell `newtype`; Rel value types are separate data types
(https://rel.relational.ai/rel/ref/declarations/entities).

## model/subtype

```
type Email <: Text
```

An `Email` is a `Text`; a `Text` becomes an `Email` by a checked
coercion. Soufflé `.type X <: number`
(https://souffle-lang.github.io/types); Third Manifesto specialization
by constraint.

## model/alias

`type Email = Text`; Soufflé equivalence types, which its docs warn let
wrong attributes bind silently (https://souffle-lang.github.io/types).
