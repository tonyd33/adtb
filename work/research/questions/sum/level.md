---
title: At which level sums exist
pick: some
options:
  value:
    title: Value-level sum, a column whose type is a union
    links:
      - {rel: prior-art, to: prior/duckdb-union}
      - {rel: prior-art, to: prior/clickhouse-variant}
  relation:
    title: Relation-level sum, an entity set partitioned into constructors
    links:
      - {rel: prior-art, to: prior/eer}
      - {rel: prior-art, to: prior/apg}
---
A sum can be the type of a value inside a row, or the shape of a set of
rows. The two support different constraints, and a design may have both.

## sum/value

The column holds one of several constructors' payloads.

```
address: US { zip: Text } | Intl { country: Text, postcode: Text }
```

Constructor fields are not keys: no `unique` on a member, no FK from a
member, no per-constructor index. Suits nested payloads.

## sum/relation

The entity set `Identity` is partitioned into `EmailPass`, `Google`, ...
Each constructor is a relation with its own keys, FKs and indexes.

```
relation identities : Identity
  key id
  unique (gid) in Google
type Session = { identity: Key identities, expires: Time }
```

The login example needs this level: uniqueness of `gid` and FKs into
identities are per-constructor or per-sum constraints.
