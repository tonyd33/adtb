---
title: Nominal types leave query results without a type name
pick: any
raised-by: [model/nominal]
options:
  anon-records:
    title: Results get anonymous structural record types
    links:
      - {rel: same-as, to: model/nominal-sums}
  declared-results:
    title: Every result shape must be declared
  fresh-names:
    title: The compiler mints a fresh nominal type per projection
  live:
    title: Live without projections that build new shapes
    excludes: [model/anon-results/anon-records, model/anon-results/declared-results, model/anon-results/fresh-names]
---
`from s in sessions select { id: s.id, expires: s.expires }` has a shape that no
declaration names.

## model/anon-results/anon-records

Stored types are nominal; anything a query builds is structural. This
is `model/nominal-sums` for records.

## model/anon-results/declared-results

```
type SessionRow = { id: Id, expires: Time }
from s in sessions select SessionRow { id: s.id, expires: s.expires }
```

## model/anon-results/fresh-names

Two queries projecting the same fields yield unequal types; a union of
them needs an explicit conversion.

## model/anon-results/live

A query returns whole stored values or scalars only.
