---
title: Whether a match over a sum may use a wildcard arm
pick: one
raised-by: [query/compile-time]
options:
  allowed:
    title: A wildcard arm covers every constructor not named
    links:
      - {rel: rivals, to: query/wildcard-hides, for: new constructors}
  scalars-only:
    title: Wildcards only on scalars; every constructor of a sum named
  marked:
    title: A wildcard must name the constructors it was written against
---
`evolution/breaks/recompile` re-checks dependents when a constructor is
added. A `_` arm absorbs it and the recheck passes silently, which is the
failure `query/wildcard-hides` records for required wildcards.

## lang/wildcard/allowed

```
match i | Google g -> g.gid | _ -> "other"
```

ML and Rust.

## lang/wildcard/scalars-only

```
match i | Google g -> g.gid | EmailPass _ -> "other"
```

Adding `Apple` breaks every match on `Identity` until it is handled.
Literal patterns still take `_`.

## lang/wildcard/marked

```
match i | Google g -> g.gid | _ of {EmailPass} -> "other"
```

The wildcard records the set it covered when checked; a constructor
added later is uncovered and fails the recheck. No precedent found.
