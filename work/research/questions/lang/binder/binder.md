---
title: The type of a constructor binder in a match arm
pick: one
raised-by: [query/construct]
options:
  ctor-row:
    title: The constructor's row type, a subtype of the sum
  fields:
    title: A plain record of the constructor's fields
  refined:
    title: The scrutinee itself, refined to the matched constructor
    links:
      - {rel: prior-art, to: prior/flix}
---
In `match i | EmailPass e -> ...`, whether `e` is a row of
`Identity.EmailPass` (with the sum's key and identity) or only its
fields decides whether `e` can be passed where an `Identity` is
expected, compared to a `Key identities`, or returned as one.

## lang/binder/ctor-row

```
match i | EmailPass e -> { owner: e.user, ident: e } | Google g -> ...   -- e : Identity.EmailPass <: Identity
```

Constructor relations are relations (`query/ctor-relations`); a binder is
one of their rows. A `Key identities.EmailPass` (`fk/to-ctor`) takes `e`.

## lang/binder/fields

```
match i | EmailPass { email, verified } -> email | Google g -> ...
```

OCaml inline records cannot escape the arm; Haskell record
constructors give fields only. The arm reuses `i` for identity.

## lang/binder/refined

```
match i | EmailPass -> i.email | Google -> ...   -- i : Identity.EmailPass
```

TypeScript narrowing, Flix restrictable variants (Madsen et al., ECOOP
2023). `query/narrowing` was defeated as a separate filter primitive;
this is typing inside a match only.
