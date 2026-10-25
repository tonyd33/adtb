---
title: Where each setting lives and when it can change
pick: some
raised-by: [ops/config/few, ops/config/derived, ops/config/many]
options:
  format:
    title: Fixed when the file is created and recorded in its header
  file:
    title: Persisted in the file and changed by a transaction
  open:
    title: Given when the file is opened; fixed for the process
  session:
    title: Set per session, transaction or call
---
SQLite splits pragmas between the file header (`page_size`,
`auto_vacuum`, `user_version`) and the connection (`cache_size`,
`busy_timeout`). A setting belongs with what it must agree with:
the page size with the data, the pool size with the host, a timeout
with the caller.

## ops/config/scope/format

Page size (deferred `layout/page-size`), checksum algorithm. Changing
one is a rewrite of the file (`ops/compact`).

## ops/config/scope/file

Settings every process opening the file must share, such as the
retention of statistics windows or a default statement limit. Moves
with the file to another host.

## ops/config/scope/open

Pool size, worker count, I/O path: properties of the host and
process.

```rust
let db = adtb::open("app.db", Options { pool: 8 << 30, workers: 16 })?;
```

## ops/config/scope/session

Limits and priority for one session or call.
