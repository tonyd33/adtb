---
title: Adding a constructor breaks stored exhaustive matches
pick: any
raised-by: [[evolution/add, query/compile-time]]
options:
  recompile:
    title: Recompile stored queries on schema change and report failures
  versioned:
    title: Queries pinned to a schema version, migrated explicitly
  rewrite:
    title: Rewrite dependent queries automatically through the change
    links:
      - {rel: prior-art, to: prior/prism-bidel}
  classify:
    title: Classify changes as breaking or dangerous and gate on it
    links:
      - {rel: prior-art, to: prior/graphql}
  live:
    title: Live with matches failing after the change
    excludes: [evolution/breaks/recompile, evolution/breaks/versioned, evolution/breaks/classify, evolution/breaks/rewrite]
---
Views, functions and application queries compiled against `Provider`
with two constructors are no longer exhaustive once `Apple` is added.

## evolution/breaks/recompile

The schema change lists every dependent query that no longer checks,
as a compiler lists non-exhaustive matches. The change is refused or
applied with the report. Postgres refuses drops with dependents but
tracks dependencies only for parsed (`BEGIN ATOMIC`) bodies.

## evolution/breaks/versioned

Each stored query names the schema version it was checked against; the
old version is served by a view until the query is updated. InVerDa
serves every version as views over shared tables, readable and
writable.

## evolution/breaks/rewrite

PRISM++ rewrites legacy queries and updates through each SMO. For an
added constructor, the rewrite cannot invent the new arm.

## evolution/breaks/classify

As graphql-js: removing a constructor is breaking and refused while
dependents exist; adding one is dangerous and reported.

## evolution/breaks/live

Dependent queries fail when next compiled.
