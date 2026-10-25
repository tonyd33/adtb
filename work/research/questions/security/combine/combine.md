---
title: How several policies on one constructor combine
pick: one
raised-by: [security/policy/predicate]
options:
  single:
    title: At most one policy per constructor and action
    links:
      - {rel: prior-art, to: prior/security-sqlserver-rls}
  allow-deny:
    title: Allows are a union, denies subtract from it
    links:
      - {rel: prior-art, to: prior/security-gel-policies}
      - {rel: prior-art, to: prior/security-pg-rls}
---
Postgres: permissive OR, restrictive AND. Gel: allows union, denies
subtract. SQL Server: a second predicate for the same operation is an
error.

## security/combine/single

Composition is written inside the one predicate.

## security/combine/allow-deny

```
policy own on invoices.(Draft | Sent) allow read = row.owner == caller.user
policy frozen on invoices.(Draft | Sent) deny read = row.account.frozen
```

Same as permissive and restrictive.
