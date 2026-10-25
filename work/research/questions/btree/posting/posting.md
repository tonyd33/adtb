---
title: How entries sharing a key are stored on a leaf
pick: one
raised-by: [btree/structure/blink, btree/structure/bw]
options:
  plain:
    title: One physical tuple per entry, key repeated
  posting-pairs:
    title: Key once, then a sorted array of constructor and tid pairs
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  posting-grouped:
    title: Key once, then per constructor its id, a count and its tids
    links:
      - {rel: prior-art, to: prior/btree-graefe}
---
Equal keys come from non-unique indexes (`index (user)` on `identities`)
and, in unique ones, from versions of one row awaiting vacuum,
including a retag's old and new entries (`engine/retag-snapshot/two-entries`).

## btree/posting/plain

Every entry carries its key. Simplest page-space accounting; LP_DEAD
style kill bits are per entry.

## btree/posting/posting-pairs

Postgres 13 deduplication with the TID array widened to `(ctor, tid)`:
merged lazily only when a leaf would otherwise split; an insert
overlapping a posting list swaps its pair with the list's maximum in
one atomic action. Postgres refuses deduplication where equal values
differ in bytes (floats, numeric, nondeterministic collations).

```
posting tuple: hdr | key | n u12 | (ctor u32, tid 6) * n      -- 10 B per row
user_id 3: [(EmailPass,(12,3)), (EmailPass,(40,1)), (Google,(4,1))]
```

## btree/posting/posting-grouped

Härder's combined image (Graefe sec 5.8): counters per table under one
key. The constructor id is paid once per run.

```
grouped tuple: hdr | key | g u8 | (ctor, n u16, tid 6 * n) * g  -- 6 B per row
user_id 3: EmailPass 2 [(12,3),(40,1)]  Google 1 [(4,1)]
```

Removing one tid shifts its group; a retag moves a pair between groups.
