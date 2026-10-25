---
title: How a relation-level sum is laid out in relational tables
pick: one
raised-by: [sum/relation]
options:
  tag-variants:
    title: Parent table with stored tag, one table per constructor
  variants-only:
    title: Constructor tables only, the sum a union view with derived tag
  wide:
    title: One wide table, CHECKs tie nullable columns to the tag
  jsonb:
    title: Shared columns plus a jsonb payload per constructor
  native:
    title: Sum is a first-class schema object enforced by the engine
    needs: [substrate/new-engine]
---
The encoding a compiler or extension emits for a relation-level sum.
For a new engine it is the logical layout; storage is decided separately.

## encoding/tag-variants

```sql
CREATE TABLE identities (
  id   bigint PRIMARY KEY,
  kind text NOT NULL CHECK (kind IN ('email_pass','google')),
  UNIQUE (id, kind)
);
CREATE TABLE google (
  identity_id bigint PRIMARY KEY,
  kind text NOT NULL DEFAULT 'google' CHECK (kind = 'google'),
  gid  text NOT NULL UNIQUE,
  FOREIGN KEY (identity_id, kind) REFERENCES identities (id, kind)
);
```

Variant matches tag; no identity has two variants; every field NOT NULL.
FKs to the sum target `identities`, to a constructor target its table.
Totality (every identity has its variant row) is not declarative.

## encoding/variants-only

```sql
CREATE TABLE google (id bigint PRIMARY KEY, gid text NOT NULL UNIQUE);
CREATE TABLE email_pass (id bigint PRIMARY KEY, email text NOT NULL UNIQUE,
                         hash text NOT NULL);
CREATE VIEW identities AS
  SELECT id, 'google' AS kind FROM google
  UNION ALL SELECT id, 'email_pass' FROM email_pass;
```

Totality holds by construction: an identity is a variant row. Exclusivity
(no `id` in two tables) and FKs to the sum have no single table to rest
on. Adding a claim table `(id PK, kind)` that variants reference turns
this back into `encoding/tag-variants`.

## encoding/wide

```sql
CREATE TABLE identities (
  id bigint PRIMARY KEY,
  kind text NOT NULL,
  email text, hash text, gid text,
  CHECK (CASE kind
    WHEN 'email_pass' THEN email IS NOT NULL AND hash IS NOT NULL AND gid IS NULL
    WHEN 'google'     THEN gid IS NOT NULL AND email IS NULL AND hash IS NULL
  END)
);
CREATE UNIQUE INDEX ON identities (gid) WHERE kind = 'google';
```

Exclusive and total by row. Per-constructor uniqueness via partial
indexes. Postgres list partitioning by `kind` does not help: partitions
share the parent's columns, and a unique constraint on a partitioned
table must include the partition key.

## encoding/jsonb

```sql
CREATE TABLE identities (id bigint PRIMARY KEY, kind text NOT NULL,
                         data jsonb NOT NULL);
```

Fields indexable through expression indexes or generated columns.
Loses planner statistics on contents, FKs and NOT NULL on inner fields,
shape validation, and space to repeated keys.

## encoding/native

```
type Identity = EmailPass { email: Text, hash: Text }
              | Google    { gid: Text }
relation identities : Identity
  key id
  unique (email) in EmailPass
  unique (gid) in Google
```

No parent table, tag column or emulation. The engine enforces
exclusivity, totality and FKs to the sum, a constructor or a subset of
constructors. Totality holds by construction: a row of `identities` is a
row of one constructor.
