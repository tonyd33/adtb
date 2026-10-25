---
title: Removing a constructor strands its rows and references
pick: any
raised-by: [evolution/remove]
options:
  restrict:
    title: Refuse while rows or references exist
  merge-first:
    title: Require migrating its rows into another constructor first
  reserve:
    title: Reserve the removed name against reuse
  deprecate:
    title: Deprecate first, remove in a later version
  live:
    title: Live with cascading deletes
    excludes: [evolution/remove-rows/restrict, evolution/remove-rows/merge-first]
---
`Facebook` rows exist; FKs target `identities.Facebook`; queries name it.
Postgres cannot drop an enum value and refuses drops with dependents
unless `CASCADE`.

## evolution/remove-rows/restrict

`RESTRICT` semantics on rows, references and dependent queries.

## evolution/remove-rows/merge-first

Removal is a merge into a surviving constructor followed by dropping
the empty one.

## evolution/remove-rows/reserve

As protobuf `reserved`: the name and constructor id cannot return with
a different meaning.

## evolution/remove-rows/deprecate

New writes refused, reads allowed, queries warned, then removal.

## evolution/remove-rows/live

Remove deletes the rows and cascades.
