---
title: Which collection kinds a field may hold
pick: some
raised-by: [model/scalar-collections, model/nf2]
options:
  set-kind:
    title: Set
  list-kind:
    title: List
  bag-kind:
    title: Bag
  map-kind:
    title: Map from a key type to a value type
---
NRC treats set, bag and list uniformly as collection monads; each has
its own equality, which keys and indexes over the field inherit.

## model/set-kind

`tags: Set Text`; a relation-valued attribute when the element is a record.

## model/list-kind

`lines: List Line`; ordered, duplicates kept.

## model/bag-kind

`Bag Text`; unordered, duplicates kept.

## model/map-kind

`prices: Map Currency Decimal`; equivalent to a set of records keyed
by the map key.
