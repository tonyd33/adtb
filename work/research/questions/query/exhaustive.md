---
title: How exhaustiveness of a match is checked
pick: one
raised-by: [query/construct]
options:
  compile-time:
    title: Compile-time check against the declared constructors
    needs: [sum/closed]
  wildcard:
    title: Every match must end in a wildcard arm
  runtime:
    title: Error at run time on a row no arm matches
  unchecked:
    title: Unmatched rows yield NULL or are dropped
  warn:
    title: Non-exhaustive match compiles with a warning
  owner-split:
    title: Exhaustive inside the schema owner, wildcard required outside
    needs: [sum/owner-closed]
  unknown-arm:
    title: A generated unknown arm every client must handle
    links:
      - {rel: prior-art, to: prior/graphql}
---
What happens when a match omits a constructor.

## query/compile-time

Flix rejects non-exhaustive matches.

`match i | EmailPass e -> ...` without `Google` is rejected when the
query is compiled.

## query/wildcard

```
match i | EmailPass e -> e.email | _ -> null
```

Required syntactically; the only option that works for open sums.

## query/runtime

The compiled query raises on an unmatched row, e.g. via a `CASE` whose
`ELSE` calls an error function.

## query/unchecked

SQL `CASE` semantics: no `ELSE` gives NULL, as DuckDB `union_extract`
and Oracle `TREAT` do. A NULL from no arm is indistinguishable from a
real NULL, and aggregates drop those rows.

## query/warn

As GHC and OCaml report incomplete patterns (unverified for a query
language).

## query/owner-split

Rust `#[non_exhaustive]`: inside the defining crate matches must be
exhaustive; outside, `_` is required.

## query/unknown-arm

Results carry an `unknown` constructor for members added after the
client was compiled, as Apollo iOS generates for unions.
