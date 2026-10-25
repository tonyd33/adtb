---
title: How much of the engine is configurable
pick: one
raised-by: [substrate/new-engine]
options:
  few:
    title: A short fixed list of resource sizes; everything else is fixed behaviour
    links:
      - {rel: prior-art, to: prior/ops-tigerbeetle}
  derived:
    title: Resource sizes derived from the host by default, overridable; no tuning knobs
  many:
    title: A setting per tunable, each with a context saying when it can change
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
---
Postgres has hundreds of settings across seven contexts (internal,
postmaster, sighup, backend, superuser-backend, superuser, user).
Many exist because behaviour was made optional (isolation levels, NULL
handling, constraint timing); here those are settled single
behaviours: one isolation level, no NULL, multiple assignment instead
of deferred constraints. What remains is resources: page pool size,
query memory, worker threads, log size, timeouts.

## ops/config/few

TigerBeetle: memory sized at startup from a few flags. A setting is
added only when no fixed value is right for both a phone and a server.

## ops/config/derived

DuckDB: `memory_limit` 80% of RAM, `threads` the core count, by
default.

```
$ adt serve app.db                  # pool = 25% RAM, workers = cores
$ adt serve app.db --pool 8GiB
```

## ops/config/many

The Postgres model, `pg_settings` with a context per setting.
