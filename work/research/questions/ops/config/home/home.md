---
title: How settings are read and changed
pick: one
raised-by: [ops/config/few, ops/config/derived, ops/config/many]
options:
  relation:
    title: Settings are a system relation, read by queries and changed by writes
    needs: [engine/catalog-relations]
  file:
    title: A configuration file read at start, shown read-only in a system relation
  pragma:
    title: A command form separate from queries and writes
    links:
      - {rel: prior-art, to: prior/ops-sqlite-pragma}
---
Settled: the catalog is relations. Settings could be one more.

## ops/config/home/relation

```
from s in sys::settings where s.name == "statement_limit" select s
update s in sys::settings where s.name == "statement_limit" set { s with value: 5s }
```

A change is transactional and logged like any write; under
`ops/config/scope/file` it moves with the file.

## ops/config/home/file

`postgresql.conf` plus `pg_settings`. Restart or reload to change.

## ops/config/home/pragma

SQLite `PRAGMA cache_size = 1000`; side-effect-free pragmas also read
as table functions.
