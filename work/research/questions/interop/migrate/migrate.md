---
title: How a SQL database is migrated in
pick: one
options:
  tool:
    title: An offline tool reads a SQL catalog and data and proposes a schema and a load
  attach:
    title: The engine reads a live SQL database as foreign relations and loads by query
    needs: [interop/foreign/attach]
  none:
    title: No migration support beyond file loads
---
Users arrive with Postgres or MySQL schemas that encode sums as
nullable columns, type columns and polymorphic ids
(`encoding/` records why each is wrong). There is no SQL surface
(`query/plain-sql` defeated), so whatever reads SQL is outside the
language.

## interop/migrate/tool

```
$ adt import-sql postgres://old/app --propose > schema/0001.adt
# edit the proposal: confirm sums, name constructors
$ adt import-sql postgres://old/app --load schema/0001.adt app.db
```

The tool emits operators and a load script in the native notation;
the engine never parses SQL. Prisma `db pull` introspects a database
into a schema file (https://www.prisma.io/docs/orm/prisma-schema/introspection,
unverified).

## interop/migrate/attach

```
attach postgres "postgres://old/app" as old
insert identities from (from r in old::identities select match r.kind | ...)
```

`postgres_fdw` `IMPORT FOREIGN SCHEMA`
(https://www.postgresql.org/docs/current/postgres-fdw.html). Brings a
SQL type mapping into the engine.

## interop/migrate/none

Export CSV or Parquet from the old database, write the schema by
hand, load with `interop/load`.
