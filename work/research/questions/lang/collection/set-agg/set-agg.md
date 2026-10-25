---
title: Set semantics collapses the values an aggregate sums
pick: any
raised-by: [lang/collection/sets]
options:
  over-binders:
    title: Aggregates range over bindings, not projected values
  live:
    title: Live with sums over distinct values
    excludes: [lang/collection/set-agg/over-binders]
---
`sum(from e in employees select e.salary)` over a set adds each distinct
salary once. Two employees on the same salary count as one.

## lang/collection/set-agg/over-binders

```
sum(e.salary for e in employees)
```

The aggregate sees one item per binding, keyed by the binder, as Morel
`sum over e.salary` and Tutorial D `SUMMARIZE` over a keyed relation
(unverified that Rel deduplicates first).

## lang/collection/set-agg/live

Project the key alongside the value to keep duplicates.
