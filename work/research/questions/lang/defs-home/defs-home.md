---
title: Where named queries and functions live
pick: some
options:
  schema:
    title: Schema objects, rechecked with the schema
  bundle:
    title: Client bundles registered together with their queries
    needs: [evolution/breaks/invisible/registered]
---
A function used by registered queries is itself a dependent of the
schema. Whether it is shared schema state or client code decides who can
change it and what a schema change rechecks.

## lang/defs-home/schema

```
module auth
type Identity = EmailPass { ... } | Google { ... }
fn logins (u: Key users) -> Set Text = ...
```

Postgres functions and views, EdgeQL functions in the schema module.
Changing one rechecks every dependent query.

## lang/defs-home/bundle

```
bundle billing pins 8 (a91c...)
fn overdue () -> Set Invoice = ...
query list_overdue () -> Set Invoice = from i in overdue() select i
```

The application owns its helpers; registration checks the bundle as a
unit, and a schema change rechecks each bundle.
