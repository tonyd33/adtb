---
title: A function over a whole group cannot be split across workers
pick: any
raised-by: [interop/uda/collection-fn]
options:
  fold-only:
    title: Collection functions defined by a fold with a declared merge are split; others are not
  serial:
    title: Each group's function runs on one worker after its rows are gathered
  live:
    title: Live with aggregates that gather every group in memory
    excludes: [interop/uda/parallel/fold-only, interop/uda/parallel/serial]
---
`median(o.total for o in g)` needs all of `g`. Bounded per-query memory
(`buffer/memory/split`) caps the largest group.

## interop/uda/parallel/fold-only

Reintroduces the state, step and merge of `fold-decl` as a body form.

## interop/uda/parallel/serial

Groups are spilled and gathered by key, then each is a call.

## interop/uda/parallel/live

A large group fails the query.
