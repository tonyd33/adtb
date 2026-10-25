---
title: What the TID in a union index entry names under in-heap versions
pick: one
raised-by: [engine/in-heap]
options:
  version-tid:
    title: The physical slot of a version, new entries when a version moves page
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  row-id:
    title: A stable per-heap row id mapped to the chain head through an indirection table
    links:
      - {rel: prior-art, to: prior/mvcc-wu-2017}
links:
  - {rel: prior-art, to: prior/mvcc-wu-2017}
---
The entry is settled as `(key, ctor, tid)`. Index entries carry no
version information: a probe may hit invisible versions, never miss a
visible one. A retag changes `ctor`, so it writes every union index
over the sum under either option.

## mvcc/version-tid

`tid = (block u32, offset u16)` of a version in heap `ctor`. A cold
update (any indexed field changed, or no room on the page) inserts a
new entry into every index whose constructor set contains the heap.
Paired with `mvcc/hot` for same-page updates.

## mvcc/row-id

Wu et al.'s logical TupleId: `tid` is a 64-bit row id; each heap keeps
`row_id -> head (block, offset)`. Updates that change no key touch only
the map; secondary entries change only when their key changes. One
extra lookup per probe; the map is itself versioned state to log and
vacuum. Logical pointers gave 45% more throughput with 20 secondary
indexes under high contention (Wu et al. sect. 7.5).

```
union index identity_user (user) entry (3, Google, row 812)
heap identities.Google map: row 812 -> (block 40, offset 7)
```
