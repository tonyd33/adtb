---
title: Functorial migration has no native sums and no row routing
pick: any
raised-by: [evolution/functorial]
options:
  query-migration:
    title: Migrations as queries (Delta after Pi with where clauses)
  encode:
    title: Encode each sum as one entity per constructor plus a parent
  live:
    title: Live with functorial migration covering add and merge only
    excludes: [evolution/functorial-split/query-migration, evolution/functorial-split/encode]
links:
  - {rel: prior-art, to: prior/cql}
---
CQL schemas have no coproduct construct. A functor maps entities, not
rows, so split by value is not a schema mapping. Sigma does not preserve
type algebras, so attribute constraints can break under merge.

## evolution/functorial-split/query-migration

Every CQL uber-flower query evaluates as Delta after Pi; a `where` on
attributes routes rows (unverified for attribute constants).

## evolution/functorial-split/encode

Sums become entities with a parent, migrated by ordinary functors.
Exclusivity and totality are then constraints outside the functor.

## evolution/functorial-split/live

Use functors where they fit, another method for the rest.
