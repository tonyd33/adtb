---
title: How client queries become registered
pick: some
raised-by: [evolution/breaks/invisible/registered]
options:
  build-time:
    title: A build step sends query files to the engine, which checks and stores them
    links:
      - {rel: prior-art, to: prior/surface-persisted-queries}
      - {rel: prior-art, to: prior/surface-sqlc}
  schema-resident:
    title: Queries are named definitions in the schema, deployed by migrations
    links:
      - {rel: prior-art, to: prior/surface-convex}
  lazy:
    title: A query registers itself on first execution
    assessment: {grade: hack, why: a query not yet run is invisible to the recheck it exists for}
    links:
      - {rel: prior-art, to: prior/surface-persisted-queries}
---
A registered query is one the engine has parsed and type-checked and
will recheck on every schema change.

## surface/registration/build-time

```
-- bundles/web.adb
bundle web-2.3 pins 8 (a91c...)
import auth
query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> ByEmail { email: e.email }
    | Google g    -> ByGoogle { gid: g.gid }
```

```
$ adt register bundles/web.adb
login_key  ok  #7f3c
```

Relay persisted queries, sqlc, Gel `generate queries`. The client
sends `#7f3c` and parameters.

## surface/registration/schema-resident

```
-- 0009
define fn login_key (u: Key users) -> Set Text = ...
```

Queries live in the schema and change only through migrations, like
stored functions. Convex functions are deployed with the backend.

## surface/registration/lazy

Apollo APQ: hash miss, then the client resends the text.
