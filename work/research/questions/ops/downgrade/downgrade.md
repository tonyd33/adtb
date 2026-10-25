---
title: Whether a file can go back to an older engine release
pick: one
raised-by: [layout/format-version/read-old]
options:
  none:
    title: Forward only; a file opened by a new release may be unreadable by older ones
  finalize:
    title: A new release writes only the old format until an operator finalizes the upgrade
    links:
      - {rel: prior-art, to: prior/ops-crdb}
  write-old:
    title: A file can be told to keep writing an older format version indefinitely
---
Settled: new binaries read old formats. Downgrade is the reverse
question, and decides whether trying a new release is reversible.

## ops/downgrade/none

TigerBeetle states forward migration only. Back out by restoring a
backup.

## ops/downgrade/finalize

CockroachDB: a mixed-version cluster runs on the old version's
features until finalization; rollback possible until then. Here the
file header records the highest format in use; until finalize no page
is written in a format the previous release cannot read.

```
$ adt upgrade app.db --finalize
from f in sys::format
select { min_page_format: f.min_page_format, max_page_format: f.max_page_format,
         finalized: f.finalized }
```

## ops/downgrade/write-old

DuckDB `STORAGE_VERSION`: create or keep a file in an older format so
older readers can open it, giving up new features.
