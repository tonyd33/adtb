---
title: How an index-only scan decides an entry is visible without the row
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  vm-per-heap:
    title: Each constructor heap has a visibility map, chosen by the entry's constructor
    needs: [engine/in-heap]
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  page-maxtrx:
    title: Each index page records its newest writer, older than every snapshot means all visible
    needs: [engine/undo-store]
  entry-visibility:
    title: Entries carry creating and deleting transaction
    needs: [engine/index-visibility]
  none:
    title: Every index scan fetches the row
---
`from i in identities where i.user == u select i.id` is answerable from
`index (user, id)` on `identities` alone if the engine knows each entry's
row version is visible to the snapshot.

## btree/ios/vm-per-heap

Postgres: two bits per heap page (all-visible, all-frozen), set only by
vacuum, cleared by any write to the page. The entry's `ctor` picks the
heap's map, `tid` the bit; one map lookup per entry, usually cached.
Index-only scans must keep the leaf pinned so vacuum cannot recycle a
TID under them.
https://www.postgresql.org/docs/current/storage-vm.html

```
entry (3, Google, (4,1), id=7)  ->  vm[Google].bit(block 4)  set: return 7
```

## btree/ios/page-maxtrx

InnoDB secondary index pages store `PAGE_MAX_TRX_ID`; if it precedes
the oldest read view the page's entries are visible, else each row is
checked through the clustered index (unverified).

## btree/ios/entry-visibility

Under `engine/index-visibility` the entry answers by itself; entries
grow by two transaction ids (16 bytes) and every update writes the old
entry.

## btree/ios/none

Simplest; costs a heap fetch per entry.
