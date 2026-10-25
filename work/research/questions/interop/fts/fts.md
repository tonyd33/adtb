---
title: Where full-text search lives
pick: one
options:
  derived:
    title: An inverted index as a derived term relation with a B-tree; tokenizers are functions
    needs: [interop/index-am/derived]
  tantivy:
    title: Tantivy embedded as an index kind over a constructor set
    links:
      - {rel: prior-art, to: prior/interop-tantivy}
  ext:
    title: An extension, not core
  out:
    title: Out of scope; search runs in an external engine fed by change data
---
Search over `docs.(Article | Note)` by `body`. SQLite FTS5 is a virtual
table over shadow tables (https://www.sqlite.org/vtab.html).

## interop/fts/derived

```
add text index (body) in docs.(Article | Note) tokenize english
from d in docs.(Article | Note) where d.body matches "sum & types" select d
```

Transactional and replicated for free; phrase and BM25 scoring are
queries over the term relation and per-term statistics. Posting lists
compress only as well as B-tree posting pairs
(`btree/posting/posting-pairs`).

## interop/fts/tantivy

Tokenizers, BM25, phrase queries, segment merging
(https://github.com/quickwit-oss/tantivy). ParadeDB embeds it in
Postgres.

## interop/fts/ext

Through `interop/index-am`.

## interop/fts/out

Elasticsearch or Tantivy fed by `repl/cdc`.
