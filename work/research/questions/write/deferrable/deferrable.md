---
title: Which constraint kinds may be deferred to commit
pick: any
raised-by: [constraint/timing/deferrable]
options:
  fk:
    title: Foreign keys
    links:
      - {rel: prior-art, to: prior/write-pg-constraints}
  unique:
    title: Keys and unique indexes over a constructor set
  row-check:
    title: Row-level checks on fields
---
`requires` participation is deferred by ruling. Exclusivity and
totality are structural under one heap per constructor and never
checked. This bounds which declarations may say `deferred`. Postgres
defers only UNIQUE, PRIMARY KEY, REFERENCES and EXCLUDE; CHECK and NOT
NULL are checked per row.

```
identity: Key identities deferred
unique (gid) in Google deferred
```

## write/deferrable/fk

Cyclic references (`primary: Key identities` on `users`, `user: Key
users` on `identities`) need it without `write/unit/multi`.

## write/deferrable/unique

Swapping two `gid` values in two statements. Postgres allows it.

## write/deferrable/row-check

A row's own fields cannot be made consistent by another row, so
deferral gains nothing. Postgres refuses it.
