---
title: A match lowered as a disjoint union can yield equal rows from two arms
pick: any
raised-by: [model/sets]
options:
  dedupe:
    title: Results are deduplicated after the union
  keep-tag:
    title: A match result keeps the source tag so arms never collide
  prove-disjoint:
    title: The compiler proves arm outputs disjoint from keys and rejects otherwise
  live:
    title: Live with match results that are bags
    excludes: [model/dup-arms/dedupe, model/dup-arms/keep-tag, model/dup-arms/prove-disjoint]
---
`query/union-all` is accepted. Under set semantics

```
from i in identities select match i
  | EmailPass e -> e.user
  | Google g    -> g.user
```

returns user 3 once even when both arms produce it, so the union is not
disjoint once arms project away the tag.

## model/dup-arms/dedupe

The plan is a union all followed by a distinct whenever the output
contains no key of the source sum.

## model/dup-arms/keep-tag

The result carries the source row's identity or tag as hidden columns,
so it is a disjoint union of sets; the user sees duplicates only by
projecting them away explicitly.

## model/dup-arms/prove-disjoint

An arm output containing the source key is disjoint by construction;
other matches need an explicit `distinct` or are rejected.

## model/dup-arms/live

A match is the one bag-valued construct in a set-valued language.
