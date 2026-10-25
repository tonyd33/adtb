---
title: Which pattern forms a match arm may use
pick: any
raised-by: [query/construct]
options:
  nested:
    title: Nested constructor patterns on value-level sum fields
  guards:
    title: Guards on arms
  or-patterns:
    title: Or-patterns over several constructors
  literals:
    title: Literal patterns on scalars
  record:
    title: Record patterns binding fields by name
---
Beyond one constructor and one binder per arm. Each form changes what
the exhaustiveness check of `query/compile-time` must decide.

## lang/patterns/nested

```
match o
  | Shipped { via: Courier c } -> c.name
  | Shipped { via: Post p }    -> p.tracking
  | Pending _                  -> "pending"
```

Coverage over nested sums is the usual ML algorithm (Maranget, "Warnings
for pattern matching", JFP 2007).

## lang/patterns/guards

```
match i | Google g if g.domain == "corp" -> "staff" | Google _ -> "guest" | EmailPass _ -> "guest"
```

## lang/patterns/or-patterns

```
match i | Google g | Apple g -> g.sub | EmailPass e -> e.email
```

Rust and OCaml or-patterns: every alternative binds the same names.

## lang/patterns/literals

```
match c.country | "US" -> 0 | "CA" -> 1 | _ -> 2
```

Scalar domains are open, so a literal match needs a wildcard arm.

## lang/patterns/record

```
match i | EmailPass { email, verified: Yes } -> email | ...
```

Field punning as OCaml and Rust struct patterns.
