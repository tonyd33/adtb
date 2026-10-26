---
title: "Whether an index re-scoped by split, merge or nest keeps its id"
pick: one
options:
  keeps:
    title: "It keeps its id; its constructor set and fields change"
  new:
    title: "It is a new index with a new id, created after the operator's constructors; the old one is dropped"
---
Raised by `id-assignment-gaps`. A split, merge or nest rewrites the
uniques and indexes over the constructors it replaces (`unique (gid)
in Google` and `unique (sub) in Apple` become `unique (provider, sub)
in Social`). §1.2 gives ids only to a split's `unique` and `index`
clauses. The canonical name of a unique uses its id
(`m::r.unique#n`), so whether the id survives decides whether that
name does.

## spec/ids/rescoped-index/keeps

An index carried by the operator keeps its id, even when its fields
change; uniques that merge into one keep the smallest id and the
others are dropped. A violation report names the unique as before.

## spec/ids/rescoped-index/new

Every re-scoped index is a new object, taking ids after the
operator's constructors and fields in the order of the clauses it
replaces. Its old row gets `dropped`. A unique's canonical name
changes across the operator.
