---
title: Application queries are invisible to recompilation
pick: any
raised-by: [evolution/breaks/recompile]
options:
  registered:
    title: Persisted, registered queries the database checks
  live:
    title: Live with checking only stored queries
    excludes: [evolution/breaks/invisible/registered]
---
Views and stored functions are recompiled; queries in applications are
not, which is why GraphQL classes union additions as dangerous rather
than breaking.

## evolution/breaks/invisible/registered

Clients register parsed queries, as GraphQL persisted queries do; the
schema change checks them too.

## evolution/breaks/invisible/live

External clients rely on wildcards or unknown arms.
