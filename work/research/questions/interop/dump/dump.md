---
title: How a database is dumped and restored
pick: some
options:
  logical:
    title: A logical dump of the schema and the rows of every relation
    links:
      - {rel: prior-art, to: prior/interop-pg-dump}
      - {rel: prior-art, to: prior/interop-gel-dump}
  physical:
    title: A consistent copy of the single file
    links:
      - {rel: same-as, to: repl/backup/snapshot-copy}
      - {rel: prior-art, to: prior/interop-sqlite-vtab}
---
A physical copy is tied to the page format and engine release
(`layout/format-version/read-old` reads old formats forward only); a
logical dump survives format changes, moves data between engine
versions, and is reviewable. Physical copy is `repl/backup`.

## interop/dump/logical

```
$ adt dump app.db > app.dump
$ adt restore app.dump new.db
```

## interop/dump/physical

```
copy database to "app-copy.db"   -- one snapshot, dead versions dropped
```

SQLite `VACUUM INTO` (https://www.sqlite.org/lang_vacuum.html).
