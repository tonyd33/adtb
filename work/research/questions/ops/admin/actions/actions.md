---
title: The form of an operational action in the language
pick: one
raised-by: [ops/admin/in-language]
options:
  system-writes:
    title: An action is a write to a system relation
  commands:
    title: Actions are a small set of statements beside queries and writes
    links:
      - {rel: prior-art, to: prior/ops-sqlite-pragma}
  procedures:
    title: Actions are engine-provided procedures called by name
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
---
Settled: queries are pure and a write is a change-set value a
transaction applies.

## ops/admin/actions/system-writes

```
delete s in sys::sessions where s.id == 4471     -- terminate
insert sys::vacuum_requests { sum: types.Identity }
update s in sys::settings where s.name == "statement_limit" set { s with value: 5s }
```

One mechanism; privileges on system relations are the access control.

## ops/admin/actions/commands

`vacuum`, `cancel`, `check`, `compact`, `reindex` as statements,
typed and checked, returning relations (a vacuum returns its summary).

## ops/admin/actions/procedures

Postgres `pg_cancel_backend(pid)`, `pg_terminate_backend(pid, timeout)`.
Needs a procedure concept the language does not otherwise have.
