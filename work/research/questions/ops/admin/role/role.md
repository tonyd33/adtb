---
title: What privileges operations need
pick: one
raised-by: [surface/adhoc/scoped]
options:
  superuser:
    title: One admin role may do every operation
  capabilities:
    title: Separate privileges to monitor, cancel, maintain and configure
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
---
Proposed: ad hoc queries run only in a separate role, outside the
registered guarantee. Operations are ad hoc by nature; the question is
whether that role is one thing.

## ops/admin/role/superuser

Simple. A monitoring agent with read access to statistics can also
compact the file.

## ops/admin/role/capabilities

Postgres `pg_monitor`, `pg_signal_backend`, `pg_maintain` (unverified
for the last). An embedded caller holds every capability; the server
grants them to roles.

```
grant monitor, cancel to oncall
grant maintain to dba
```
