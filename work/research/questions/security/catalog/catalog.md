---
title: What the catalog shows a role
pick: one
raised-by: [engine/catalog-relations]
options:
  open:
    title: Every role reads the whole catalog
  filtered:
    title: Catalog relations have policies; a role sees objects it holds a grant on
    needs: [security/policy/predicate]
---
The catalog is relations read with the same language: constructor
names, policy text, registered query text, grants.

## security/catalog/open

Postgres `pg_catalog` is readable by every role (unverified).

## security/catalog/filtered

```
from c in catalog::constructors where c.sum == "Identity" select c.name
-- support: EmailPass, Google
```

Needed by `security/grain/partial/role-view`.
