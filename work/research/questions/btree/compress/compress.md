---
title: Key compression in ctor-set index pages
pick: any
raised-by: [encoding/native-cost/union-index]
options:
  suffix-attr:
    title: Pivot keys keep only the attributes needed to separate the split halves
    needs-one-of: [btree/structure/blink, btree/structure/bw]
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  suffix-byte:
    title: Pivot keys truncated to the shortest separating byte string
    needs-one-of: [btree/structure/blink, btree/structure/bw]
    needs: [btree/keyenc/memcmp]
    links:
      - {rel: prior-art, to: prior/btree-graefe}
  prefix-fence:
    title: Bytes shared by a page's two fence keys stored once per page
    needs-one-of: [btree/structure/blink, btree/structure/bw]
    needs: [btree/keyenc/memcmp]
    links:
      - {rel: prior-art, to: prior/btree-graefe}
  slot-head:
    title: The first bytes of each normalized key kept in its slot
    needs-one-of: [btree/keyenc/memcmp, btree/keyenc/hybrid]
    links:
      - {rel: prior-art, to: prior/btree-graefe}
  singleton-elide:
    title: Indexes over one constructor store no constructor id
---
Layers that stack. Each changes page bytes only, never which entries
exist.

## btree/compress/suffix-attr

Postgres: at a leaf split the new high key, reused as the parent's
downlink, drops trailing attributes; dropped ones read as minus
infinity. In a union index `(ctor, tid)` is the trailing part and is
nearly always dropped, so pivots cost no constructor bytes.

```
left last (7, Google, (4,1))  right first (8, EmailPass, (2,9))  pivot (8)
```

## btree/compress/suffix-byte

Bayer and Unterauer's prefix B-tree: separator is the shortest byte
string between the last left and first right key; Postgres stops at
whole attributes. Graefe sec 3.5.

## btree/compress/prefix-fence

Each page keeps low and high fence keys; their common prefix is
removed from every key and skipped in comparisons. Basing it on fences,
not current keys, means an insert never reformats the page (Graefe).

```
fences ("smith, ja" .. "smith, jo")   prefix "smith, j"   keys "ack", "ane"
```

## btree/compress/slot-head

Graefe's "poor man's normalized key": 2-4 leading key bytes beside
each slot so binary search mostly avoids touching the tuple.

## btree/compress/singleton-elide

`unique (gid) in Google` holds `(gid, tid)`; the constructor is
implied by the index's set. Every reader of the entry gets its
constructor from index metadata.
