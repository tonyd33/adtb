---
title: A database file opened by a binary lacking an extension it uses
pick: any
raised-by: [interop/ext-pkg/linked, interop/ext-pkg/loadable]
options:
  refuse-open:
    title: Opening fails, naming the missing extension and version
  fence:
    title: The file opens; relations and indexes using the extension are unreadable and unwritable
  live:
    title: Live with indexes left stale by writes from a binary that could not maintain them
    excludes: [interop/ext-pkg/missing/refuse-open, interop/ext-pkg/missing/fence]
---
An embedded file moves between programs (`layout/files/single-file`).
A write to `docs` by a binary without the vector extension cannot
maintain the vector index.

## interop/ext-pkg/missing/refuse-open

The safe default for an embedded database.

## interop/ext-pkg/missing/fence

Administrative tools (dump, inspect) work on the rest.

## interop/ext-pkg/missing/live

The index is wrong from that write on.
