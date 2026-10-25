---
title: Where row-level authorization lives
pick: one
options:
  in-queries:
    title: In registered queries; there are no policies, only what each query selects
    needs: [security/subject/queries]
    links:
      - {rel: prior-art, to: prior/surface-convex}
  predicate:
    title: Policies are predicates attached to relations and conjoined to every read and write
    links:
      - {rel: prior-art, to: prior/security-pg-rls}
      - {rel: prior-art, to: prior/security-gel-policies}
      - {rel: prior-art, to: prior/security-sqlserver-rls}
      - {rel: prior-art, to: prior/security-oracle-vpd}
  schema-fns:
    title: Readers are granted schema functions that filter, never the relations under them
    needs: [lang/defs-home/schema]
---
With registered-only access the engine already sees every query
application code can run. Policies add a guarantee that holds whichever
query runs, including the ad hoc role's.

## security/policy/in-queries

```
query my_invoices () -> Set Invoice = from v in invoices where v.owner == caller.user select v
```

Convex. A review of the bundle is the review of its authorization; a
forgotten `where` is a leak.

## security/policy/predicate

```
policy own on invoices for read, write =
  match caller | User u -> row.owner == u.id | Service _ -> true | Anonymous -> false
```

Postgres RLS, Gel access policies, SQL Server security policies, Oracle
VPD. The planner adds the predicate under the query; registered queries
are planned with it at registration, so a policy change is a schema
change that replans dependents.

## security/policy/schema-fns

```
define fn visible_invoices () -> Set Invoice =
  from v in invoices where v.owner == caller.user select v
grant call visible_invoices to web
```

A security-barrier view as the only access path.
