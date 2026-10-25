---
title: Adding a constructor or field changes the row format
pick: any
raised-by: [[storage/wide-row, evolution/add], [storage/wide-row, evolution/move-field]]
options:
  versioned:
    title: Version the format per constructor, rewrite lazily
  missing-default:
    title: Absent trailing fields read as a stored default
  live:
    title: Live with rewriting the heap
    excludes: [storage/row-format/versioned, storage/row-format/missing-default]
---
Rows written before the change lack the new field or carry a layout
the new schema does not expect.

## storage/row-format/versioned

Each row's header names its format version; readers decode old versions.

## storage/row-format/missing-default

Postgres `atthasmissing` reads a fast-default for attributes beyond the
tuple's stored count (unverified details).

## storage/row-format/live

`ALTER` rewrites the heap, as SQL Server does when toggling sparse.
