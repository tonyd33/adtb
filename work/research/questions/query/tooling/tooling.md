---
title: SQL-only tools cannot issue a match
pick: any
raised-by: [query/construct]
options:
  sql-escape:
    title: Expose generated SQL views per sum and constructor
    needs: [query/plain-sql]
  live:
    title: Live with SQL tools seeing no sums
    excludes: [query/tooling/sql-escape]
---
BI tools, ORMs and drivers speak SQL. A match construct is invisible to
them.

## query/tooling/sql-escape

`identity` and `identity_google`, `identity_email_pass` as views, so
SQL clients read the sum as tag plus per-constructor relations.

## query/tooling/live

Clients use the native query language or nothing.
