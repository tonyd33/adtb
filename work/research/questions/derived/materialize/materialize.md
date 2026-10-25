---
title: Whether a derived relation can be stored and kept current
pick: one
raised-by: [lang/defs-home/schema]
options:
  virtual-only:
    title: Named queries are never stored; indexes are the only derived storage
  storage-choice:
    title: A derived relation is declared once and storing it is a physical choice
    links:
      - {rel: prior-art, to: prior/derived-materialize}
      - {rel: prior-art, to: prior/derived-dbsp}
      - {rel: gains, to: engine/guarantees-only}
  refresh-kind:
    title: A stored view is its own kind whose content is its last refresh
    links:
      - {rel: prior-art, to: prior/derived-pg-matview}
      - {rel: prior-art, to: prior/derived-oracle-mv}
links:
  - {rel: prior-art, to: prior/constraint-ivm}
  - {rel: variant-of, to: lang/views}
---
Views are schema functions (`lang/defs-home/schema`). Whether one
may also be stored, and whether storing it changes what it denotes.

```
fn open_by_user () -> Set { user: Key users, open: I64 } =
  from u in users
  select { user: key u, open: count(i for i in invoices.Open where i.user == key u) }
```

## derived/materialize/virtual-only

Every read of `open_by_user()` evaluates it. Derived storage is limited
to indexes (union indexes, constructor-field indexes) and whatever the
engine keeps internally for constraints (`constraint/aggregates/maintained`).
Tutorial D virtual relvars.

## derived/materialize/storage-choice

```
store open_by_user
```

Storing is an attribute like an index: queries reading `open_by_user()`
are unchanged and always see its value at their snapshot. Materialize
indexes and materialized views, Feldera `MATERIALIZED`, DBSP. Fits
`engine/guarantees-only`: the guarantee is the function's value, the
store is implementation.

## derived/materialize/refresh-kind

```
materialized open_by_user = from u in users select ...
refresh open_by_user
```

A separate kind whose content is the query's value as of its last
refresh; readers may see a stale relation by design. Postgres
`REFRESH MATERIALIZED VIEW` replaces contents whole; Oracle `ON DEMAND`.
