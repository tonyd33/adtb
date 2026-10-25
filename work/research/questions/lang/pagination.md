---
title: How results are paged
pick: some
options:
  keyset:
    title: Pages continue after a key of the last row seen
  offset:
    title: Pages skip a number of rows
---
Results are sets ordered only at the top.

## lang/keyset

`... order by u.id take 50 after cursor` where `cursor: Option (Key users)`.

## lang/offset

`take 50 skip 100`; unstable under concurrent writes.
