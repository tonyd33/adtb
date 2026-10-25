---
title: Kept history holds data that must be deleted for real
pick: any
raised-by: [derived/time-travel/versioned, derived/time-travel/everything]
options:
  excise:
    title: An operator removes matching versions from all history, irrevocably
    links:
      - {rel: prior-art, to: prior/derived-datomic-time}
      - {rel: prior-art, to: prior/derived-xtdb}
  prune:
    title: History is pruned only by age, per relation
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  live:
    title: Live with history that cannot be erased
    excludes: [derived/time-travel/erase/excise, derived/time-travel/erase/prune]
---
A right-to-erasure request names one user; their rows live in every
version since.

## derived/time-travel/erase/excise

```
excise from u in users where u.id == 7 select u
```

Datomic excision rewrites index and log, at cost proportional to the
database; XTDB `ERASE`. Past reads change, so `at version` is no longer
repeatable for those rows.

## derived/time-travel/erase/prune

MariaDB `DELETE HISTORY ... BEFORE SYSTEM_TIME`. Erasure waits for the
age limit.

## derived/time-travel/erase/live

Nothing removes a version once retained.
