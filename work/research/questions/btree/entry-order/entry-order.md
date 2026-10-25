---
title: Where the constructor id sits in an entry's sort order
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  after-key:
    title: Sort by key, then constructor, then tid
    links:
      - {rel: prior-art, to: prior/btree-graefe}
  after-tid:
    title: Sort by key, then tid, then constructor
  leading:
    title: Sort by constructor first, then key and tid
    assessment: {grade: costly, why: a probe of the whole set is one seek per constructor}
---
TIDs are per heap: `(3, 7)` in `EmailPass` and `(3, 7)` in `Google`
are different rows, so `(key, tid)` is not unique across heaps and
cannot be the B-link tiebreak by itself. The constructor id must be in
the sort key; its position decides what is contiguous.

## btree/entry-order/after-key

`(key, ctor, tid)`: Postgres's heap-TID tiebreak widened to
`(ctor, tid)`. All entries of one key are adjacent, so a uniqueness
probe reads one range; within a key, one constructor's entries are
adjacent, so a `Key identities.Google` reference seeks `(key, Google)`.
Graefe's merged indexes likewise put the index identifier after the
key fields in the normalized key.

```
(7, EmailPass, (12,3))  (7, Google, (4,1))  (8, Google, (4,2))
from i in identities.Google where i.id == 7 select i   -- seek (7, Google)
```

## btree/entry-order/after-tid

`(key, tid, ctor)`: still unique, still one range per key, but one
constructor's entries for a key interleave with others' in TID order.
Gains nothing over `after-key` unless TIDs are globally unique, in
which case `ctor` is redundant in the sort key.

## btree/entry-order/leading

`(ctor, key, tid)`: each constructor is a contiguous range, so a
dropped constructor is a range delete (`union-index/drop-ctor` notes
the conflict). A probe of a set of N constructors is N seeks: the
`encoding/native-cost/live` cost inside one tree.
