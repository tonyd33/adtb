---
title: PostgreSQL nbtree
---
Lehman and Yao B-link tree: right-link and high key per page, descents
move right on a concurrent split. Heap TID is the last key attribute,
so every entry is unique. Suffix truncation of pivots by whole
attribute. Deduplication (PG 13) into posting lists of 6-byte TIDs,
lazily before a split. LP_DEAD kill bits, bottom-up deletion (PG 14),
VACUUM in block order with a cycle id per page. Only empty pages are
deleted. 16-byte special area: prev, next, level, flags, cycle id.
Items over 1/3 page (2704 bytes at 8 KiB) refused.
https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/nbtree/README
