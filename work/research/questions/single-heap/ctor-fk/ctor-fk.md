---
title: A constructor's unique field is covered only by a partial index
pick: any
raised-by: [encoding/native-cost/single-heap]
options:
  tag-predicate:
    title: FKs may target a partial unique index whose predicate is a tag test
  composite:
    title: Full unique index on (ctor, field), FKs reference both
  live:
    title: Live with no FKs to constructor fields
    excludes: [single-heap/ctor-fk/tag-predicate, single-heap/ctor-fk/composite]
---
`unique (gid) in Google` is `UNIQUE (gid) WHERE ctor = Google`. Postgres
lets an FK reference only a non-partial unique index, and uses a partial
index for reads only when the planner proves the query implies the
predicate, which parameterized clauses defeat.
https://www.postgresql.org/docs/current/indexes-partial.html

## single-heap/ctor-fk/tag-predicate

```
FOREIGN KEY (gid) REFERENCES identities.Google (gid)
```

resolves to the partial index on `gid WHERE ctor = Google`. The engine
knows the predicate is a tag test, so the target is static.

## single-heap/ctor-fk/composite

```
UNIQUE (ctor, gid)
FOREIGN KEY ('Google', gid) REFERENCES identities (ctor, gid)
```

Uniqueness of `gid` alone is then not enforced by this index.

## single-heap/ctor-fk/live

References name the sum id only.
