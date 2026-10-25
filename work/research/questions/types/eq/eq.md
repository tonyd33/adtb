---
title: Whether == is substitutive equality on every type
pick: one
options:
  leibniz:
    title: Equal values are indistinguishable by every operation
    links:
      - {rel: gains, to: btree/keyenc/canon/canonicalize}
  per-type:
    title: Each type defines == as an equivalence that operations may see through
    links:
      - {rel: prior-art, to: prior/types-float}
      - {rel: prior-art, to: prior/types-unicode}
---
Sets deduplicate by `==` (`model/sets`), keys are unique by it, and the
optimizer replaces a value by an equal one when it joins on a key or
reads an index-only scan. Whether two `==`-equal values may still
differ under some operation decides which representative a
deduplication keeps and whether the planner's substitutions are
observable.

## types/eq/leibniz

`a == b` implies `f(a) == f(b)` for every function `f` of the language.
A value's type quotients away every distinction `==` ignores: one zero,
one NaN or none, one encoding per string, scale fixed by the type.

```
from x in [0.0, -0.0] select x          -- one row; there is one zero
from p in people select { name: p.name } -- dedup keeps no hidden choice
```

The memcmp key of a value is then a faithful encoding of it
(`btree/keyenc/canon/canonicalize` loses nothing). Haskell's `Eq`
laws state substitutivity but do not enforce it
(https://hackage.haskell.org/package/base/docs/Data-Eq.html, unverified).

## types/eq/per-type

`==` is an equivalence each base type states: `-0.0 == 0.0` with
`1/x` telling them apart, NFC and NFD text equal but with different
bytes. Postgres float and numeric, Swift `String`.

```
from x in [0.0, -0.0] select 1.0 / x    -- one row or two, depending on which zero survived
```
