---
title: Who may append operators to the schema log
pick: one
options:
  owner:
    title: One schema-owner role
  per-module:
    title: Owners per module; an operator needs ownership of every module it touches
    needs: [surface/namespace/modules]
---
The schema is the fold of its operator log (`surface/source/op-log`),
and policies, grants and schema functions live in the catalog with it.
Appending an operator can drop data, rewrite policies, or block live
bundles.

## security/schema-priv/owner

```
grant schema to deployer
```

## security/schema-priv/per-module

```
grant own auth to identity_team
grant references auth::identities.(Google | Apple) to billing_team
```

A `billing` operator adding an FK into `auth` needs a `references`
grant on the target constructor set, as Postgres `REFERENCES`.
