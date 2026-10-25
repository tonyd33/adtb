---
title: What the authored schema artifact is
pick: one
options:
  declarative:
    title: A declarative schema file stating the whole current schema
    links:
      - {rel: prior-art, to: prior/surface-gel-migrations}
      - {rel: prior-art, to: prior/surface-prisma}
      - {rel: prior-art, to: prior/surface-atlas}
  op-log:
    title: The ordered log of migration operators; the schema is its fold
    links:
      - {rel: prior-art, to: prior/prism-bidel}
  catalog:
    title: Statements against the live catalog; no authored file
---
Which artifact a developer edits and reviews, and which one the engine
treats as truth when they disagree. With operators accepted as the
change mechanism, every option still produces a chain of operators;
they differ in what is written by hand.

## surface/source/declarative

```
module auth

type Identity = EmailPass { email: Text, hash: Bytes }
              | Google    { gid: Text }
relation identities : Identity
  key id
  unique (email) in EmailPass
  unique (gid) in Google
```

The repository holds the current schema. Changes are edits to it; the
migration is derived from or checked against the diff. Gel SDL, Prisma
schema, Atlas HCL.

## surface/source/op-log

```
-- 0007
add constructor Identity.Apple { sub: Text }
add unique (sub) in identities.Apple
rename constructor Identity.Facebook to Meta
```

The repository holds only migrations; the current schema is printed by
the engine from their fold. Rails and Liquibase changelogs
(https://guides.rubyonrails.org/active_record_migrations.html).

## surface/source/catalog

```
> add constructor Identity.Apple { sub: Text }
```

Each statement is one operator applied to the live database, which
records the history. The schema is read back by introspection. SQL DDL
without a migration tool.
