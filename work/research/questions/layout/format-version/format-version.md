---
title: How the on-disk format is versioned and upgraded across engine releases
pick: one
raised-by: [substrate/new-engine]
options:
  offline:
    title: One format version per file and an offline upgrade tool between versions
  read-old:
    title: Every release reads every earlier format and writes the newest
  lazy-upgrade:
    title: Pages carry their format version and are rewritten on touch and by a background pass
---
Engine format, not user schema: page layout, tuple header, encodings.
User schema evolution is the operators.

## layout/format-version/offline

Postgres major versions: `pg_upgrade` or dump and restore (unverified
details). SQLite's header has read and write format version bytes; a
reader refuses a higher read version. https://www.sqlite.org/fileformat2.html

## layout/format-version/read-old

DuckDB from v0.10: newer releases read older files; older releases
read newer files best-effort; `STORAGE_VERSION` writes an old format
on purpose. Decoders accumulate.
https://duckdb.org/docs/2.0/internals/storage.html

## layout/format-version/lazy-upgrade

`layout/header/version` per page; a page read in an old version is
decoded by the old code and written back new; once a background pass
finishes, the file header records the minimum version present and the
old decoder can be retired. TigerBeetle instead ships past releases in
one binary and re-execs the one the cluster runs.
