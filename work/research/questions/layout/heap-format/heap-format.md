---
title: A constructor heap's row format changes when fields move
pick: one
raised-by: [[storage/table-per-ctor, evolution/move-field]]
options:
  new-heap:
    title: Write a new heap in the new format and switch to it when copied
  page-version:
    title: Each page has one format version and pages are converted on write
    needs: [layout/header/version]
  row-version:
    title: Each row names its format version
  missing-default:
    title: Rows lacking trailing fields read them as a stored default
---
Moving a field changes two constructors' schemas, so two heaps' rows.
Split, merge and nest move rows between heaps and always write new
rows. `storage/row-format` asks the same for a single wide heap.

```
move field Identity.EmailPass.created to Identity   -- every constructor heap gains created
```

## layout/heap-format/new-heap

Catalog points the constructor at heap v2; a backfill copies rows;
writes go to both or are logged and replayed, as pgroll and online
schema-change tools do (unverified as heap-level precedent). Union
index entries are rewritten to v2 TIDs.

## layout/heap-format/page-version

```
page.format = 3: EmailPass {id, user, created}   fixed slot 48 B
page.format = 4: EmailPass {id, user}            fixed slot 40 B   (created moved to Identity)
```

Every page stays uniform, so fixed slots survive. A converted page may
no longer fit its rows; overflow rows move to other pages, changing
their TIDs.

## layout/heap-format/row-version

Avro-style writer schema per row (protobuf and Avro resolve by schema
id; unverified as heap precedent). One heap mixes widths, so slotted
pages only.

## layout/heap-format/missing-default

Postgres `atthasmissing`: a tuple's stored count is lower than the
schema's; trailing attributes read as the default recorded at `ADD
COLUMN` (unverified details). Covers adding a field only, not removing
or reordering.
