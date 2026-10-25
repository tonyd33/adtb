---
title: How tenants sharing one deployment are isolated
pick: one
options:
  policy:
    title: Shared relations with a tenant field and a row policy
    needs: [security/policy/predicate]
    links:
      - {rel: prior-art, to: prior/security-sqlserver-rls}
  relation-each:
    title: Each tenant has its own relation variables of shared types
  file-each:
    title: Each tenant is its own database file
    needs: [layout/files/single-file]
  tenant-key:
    title: A declared tenant dimension that prefixes every key and scopes every FK and session
---
Isolation must cover reads, constraint collisions, conflicts and
resource use.

## security/tenancy/policy

```
policy tenant on docs for read, write = row.tenant == caller.tenant
```

SQL Server's multitenant use case. Leaks are those of `security/leak`.

## security/tenancy/relation-each

```
-- t_acme.adb
module t_acme
relation docs : Doc key slug

-- t_beta.adb
module t_beta
relation docs : Doc key slug
```

Queries are registered per module; a schema change is applied once per
tenant module.

## security/tenancy/file-each

```
$ adt open tenants/acme.db
```

One file per tenant makes isolation physical: separate keys, backups,
deletion by unlinking. Registered bundles are per file.

## security/tenancy/tenant-key

```
tenant dimension org : Key orgs
relation docs : Doc key slug          -- stored key (org, slug)
transact in org k ...         -- every read and FK scoped to k
```

The engine prefixes keys, union-index entries and FK lookups with the
tenant, so collisions, range locks and plans never cross tenants.
