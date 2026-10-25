---
title: How a union index scan avoids heap fetches for visibility
pick: one
raised-by: [engine/own-pager]
options:
  vm-per-heap:
    title: Each constructor heap keeps a visibility map, consulted at the entry's constructor and block
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  page-max-trx:
    title: Each index page records the newest writer, entries trusted when it precedes the snapshot
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  entry-stamps:
    title: Entries carry their version's stamps and are checked without the heap
    links:
      - {rel: same-as, to: engine/index-visibility}
  heap-fetch:
    title: Every entry is checked in its heap
---
Union index entries hold the constructor id, so a query that needs only
the key and the constructor, such as a match whose arms read no
fields, can be answered from the index if visibility is settled
without the heap.

```
from i in identities where i.user == u
select match i | EmailPass _ -> "password" | Google _ -> "google"
-- reads entries (3, ctor, tid) of identity_user only
```

## mvcc/vm-per-heap

Postgres: 2 bits per heap page (all-visible, all-frozen), set by vacuum,
cleared by any write to the page; four orders of magnitude smaller than
the heap. Here one map per constructor heap; the scan probes
`vm[ctor][block(tid)]`. A heap whose constructor is rarely written stays
all-visible even while siblings churn.

## mvcc/page-max-trx

InnoDB secondary index pages carry the max trx id that modified them (`PAGE_MAX_TRX_ID`, name unverified);
a reader whose snapshot is past it trusts the page's entries, else it
visits the clustered row. Works under undo-store, where heaps keep no
dead tuples.

## mvcc/entry-stamps

Index-only always, at the cost of wider entries and index writes on
every delete (`engine/index-visibility`).

## mvcc/heap-fetch

No index-only scans.
