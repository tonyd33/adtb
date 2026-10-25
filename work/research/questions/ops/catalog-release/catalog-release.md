---
title: An engine release that adds a constructor to a catalog sum breaks exhaustive matches over it
pick: any
raised-by: [engine/catalog-relations]
options:
  catalog-smo:
    title: Catalog changes in a release are schema operators, applied readers-first to bundles that read the catalog
    links:
      - {rel: variant-of, to: evolution/old-readers/readers-first}
  translated:
    title: Each release keeps earlier catalog versions readable through the operators' inverses
    links:
      - {rel: gains, to: evolution/smo}
  unregistered:
    title: Catalog and statistics relations are readable only by ad hoc queries, never registered
    needs: [surface/adhoc/scoped]
  live:
    title: Live with it
    excludes: [ops/catalog-release/catalog-smo, ops/catalog-release/translated, ops/catalog-release/unregistered]
---
Statistics, sessions, plans, wait states and settings are sums the
engine defines (`ops/explain/form/sum`, `ops/waits`). Matches name every
constructor (settled `lang/wildcard/scalars-only`), so a monitoring
bundle with

```
from s in sys::sessions
select match s.state | Running r -> r.query | Waiting w -> w.on | Idle -> "idle"
```

stops compiling when a release adds `Admitting`. The engine is the
writer of these sums, and readers-first says writers never emit what
readers cannot type; here the writer is a binary upgrade, not a
migration.

## ops/catalog-release/catalog-smo

The catalog's own schema has an operator log like a user schema. A
release ships `add constructor sys::SessionState.Admitting` as an operator; the
engine refuses to emit the constructor until every registered bundle
reading `sys::SessionState` has been repinned, as `surface/registration/order/pinned`
orders user constructors.

## ops/catalog-release/translated

The release keeps the inverse of each catalog operator; a bundle
pinned to catalog version 7 reads a translated view in which
`Admitting` maps to `Waiting`. Every catalog operator needs an inverse
with a declared loss.

## ops/catalog-release/unregistered

Catalog relations are for the admin and ad hoc role only, where the
query is checked when run. Registered monitoring queries are not
possible.

## ops/catalog-release/live

Engine upgrades may break registered queries over the catalog;
recompile reports them.
