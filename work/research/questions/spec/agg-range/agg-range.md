---
title: "What an aggregate may range over"
pick: one
options:
  any-set:
    title: "Any set expression; sum and avg refuse a generator over a select-built set"
  binders-only:
    title: "Only rows, keys and groups"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/agg-range/any-set

Any set expression; sum and avg refuse a generator over a select-built set. Count distinct must be expressible; only sum and avg lose meaning when duplicates vanish.

## spec/agg-range/binders-only

Only rows, keys and groups. Count distinct was impossible.
