---
title: A constructor cannot sit in two crossing nested subsets
pick: any
raised-by: [fk/subset/nested-sum]
options:
  dag:
    title: Allow a constructor under several intermediate sums
  live:
    title: Live with one subset hierarchy per sum
    excludes: [fk/subset/crossing/dag]
---
`OAuth (Google | Facebook)` and `Federated (Google | Saml)` both need
`Google`. A tree of nested sums has one path to each leaf.

## fk/subset/crossing/dag

Membership forms a DAG, as with Gel multiple inheritance. Matching on
an intermediate sum must say which path it means.

## fk/subset/crossing/live

Crossing subsets use `fk/subset/subset-index` or nothing.
