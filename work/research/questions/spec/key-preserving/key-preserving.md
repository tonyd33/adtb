---
title: "Which generator binders a key-preserving select may return"
pick: one
options:
  params-and-groups:
    title: "Also the binder of a generator over a Set T parameter or over a group's rows"
  relations-only:
    title: "Only binders over relation variables, constructor relations, constructor sets, backlinks and key-preserving comprehensions"
---
Raised by the normalisation work. queries.md §10.1 refuses `sum` and
`avg` over a `select` that may have merged equal values. It accepted a
`Set T` parameter directly, but not a comprehension selecting the
binder of a generator over one, nor over a group's rows.

```adt fragment
sum(x for x in (from y in amounts where y > 0 select y))   -- amounts: Set I64
```

## spec/key-preserving/params-and-groups

A `select` that is exactly the binder of a generator over a `Set T`
parameter or over a group's rows is key-preserving too. The binder
ranges over distinct elements, so selecting it merges nothing.

## spec/key-preserving/relations-only

The list stays as written. The example above is refused although it
merges nothing, and the same set is accepted when passed directly.
