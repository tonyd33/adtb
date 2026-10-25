---
title: A write that produces a row its writer could not read
pick: one
raised-by: [security/policy/predicate]
options:
  error:
    title: The change set is refused
    links:
      - {rel: prior-art, to: prior/security-gel-policies}
      - {rel: prior-art, to: prior/security-pg-rls}
  allowed:
    title: Allowed unless a separate write policy forbids it
    links:
      - {rel: prior-art, to: prior/security-sqlserver-rls}
---
Includes a retag into a constructor the writer cannot see.

## security/write-check/error

Postgres `WITH CHECK` defaults to `USING`; Gel errors on insert and
update write. A retag from `Draft` to `Archived`, invisible to clerks,
fails for a clerk.

## security/write-check/allowed

SQL Server without a block predicate: the row is written and vanishes
from the writer's view.
