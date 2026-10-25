---
title: A path through an optional reference has no row to land on
pick: any
raised-by: [[lang/nav/paths, query/optional-sum/null-ctor]]
options:
  option-typed:
    title: The path yields an Option and must be matched
  generator:
    title: An optional path is a 0..1 collection usable as a generator
    links:
      - {rel: prior-art, to: prior/lang-edgeql}
  chain:
    title: A safe-navigation operator propagates None
    needs: [lang/nav/path-option/option-typed]
  live:
    title: Live with paths only through mandatory references
    excludes: [lang/nav/path-option/option-typed, lang/nav/path-option/generator, lang/nav/path-option/chain]
---
With NULL gone, `manager : Option (Key employees)` is a sum. `e.manager.name`
has no value when `e.manager` is `None`; SQL would yield NULL.

## lang/nav/path-option/option-typed

```
from e in employees
select match e.manager | None -> "none" | Some m -> m.name
```

Paths stop at the Option; the user matches, as on any sum.

## lang/nav/path-option/generator

```
from e in employees, m in e.manager select { name: e.name, boss: m.name }
```

EdgeQL: a missing link is the empty set and a path over it drops the
row. Inner-join meaning; employees with no manager vanish.

## lang/nav/path-option/chain

```
from e in employees select e.manager?.dept?.name   -- : Option Text
```

Kotlin and C# `?.`; equivalent to nested matches with `None` arms.

## lang/nav/path-option/live

Optional references are reached by an explicit match or join.
