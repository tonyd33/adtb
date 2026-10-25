---
title: Where an aggregate inside a match applies
pick: one
raised-by: [query/construct]
options:
  per-arm:
    title: Aggregates inside an arm range over that arm's rows
  whole:
    title: Aggregates apply only outside the match, over its result
---
`count(*)` inside the `Google` arm can mean the count of Google rows or
of all rows. The lowering differs: per-arm aggregation is inside each
branch of a union, whole-match aggregation wraps it.

## query/agg-scope/per-arm

```
match i | Google g -> count(*) | EmailPass e -> count(*)
```

gives one count per constructor, a group-by on the tag.

## query/agg-scope/whole

Arms are row-level only; `count(x for x in (from i in identities select
match i | ...))` aggregates the result.
