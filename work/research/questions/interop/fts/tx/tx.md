---
title: Tantivy's segments and commits are outside the engine's log and MVCC
pick: any
raised-by: [interop/fts/tantivy]
options:
  async:
    title: The search index lags commits and is never read inside a transaction's guarantees
  pages-dir:
    title: Tantivy's Directory implemented over engine pages under redo logging
    links:
      - {rel: rivals, to: interop/fts/derived}
  live:
    title: Live with search results from another point in time
    excludes: [interop/fts/tx/async, interop/fts/tx/pages-dir]
---
Tantivy makes documents visible at its own commit
(https://github.com/quickwit-oss/tantivy); a serializable transaction
sees one snapshot (`engine/serializable-only`).

## interop/fts/tx/async

`matches` is allowed only in ad hoc queries (`surface/adhoc/scoped`)
and returns keys a query rejoins against the snapshot.

## interop/fts/tx/pages-dir

Segments become page runs in the single file; Tantivy's own commit
becomes part of the change set. Visibility per version still needs a
delete bitmap per snapshot (unverified feasibility).

## interop/fts/tx/live

A just-deleted document is found.
