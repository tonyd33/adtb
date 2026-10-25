---
title: Heap-only updates inside a constructor heap
pick: one
raised-by: [mvcc/version-tid]
options:
  per-heap:
    title: An update is heap-only when no index whose constructor set contains this heap keys a changed field
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  none:
    title: Every update inserts new union index entries
    assessment: {grade: costly, why: every field update writes every index over the constructor}
---
Postgres HOT: an update changing no indexed column that fits on the
same page gets no index entries; the old version is flagged
`HOT_UPDATED`, the new `HEAP_ONLY`, and page pruning redirects the root
line pointer. "Indexed" includes partial-index predicate columns,
which is why a single-heap sum whose constructor indexes are partial on
the tag makes every tag change cold.

## mvcc/hot/per-heap

The blocking set is per heap: the key fields of indexes whose
constructor set contains the heap's constructor. Fields compared
bitwise, so a value-level sum field changing variant is a change.
A retag is never heap-only: the row leaves the page's heap and every
union index entry changes `ctor`.

```
type Identity = EmailPass { user: Key users, email: Text, pw: Hash }
              | Google    { user: Key users, gid: Text }
relation identities : Identity
  key id
  index (user)
  unique (gid) in Google
-- blocking fields: EmailPass {id, user}; Google {id, user, gid}
update i in identities.EmailPass where i.id == 7 set EmailPass { i with pw: h }   -- heap-only
update i in identities.Google where i.id == 9 set Google { i with gid: g }        -- cold
```

Creating an index over a constructor set must handle broken HOT chains
in those heaps only (Postgres `indcheckxmin`, CREATE INDEX
CONCURRENTLY waits); heaps outside the set are unaffected.

## mvcc/hot/none

Simpler; vacuum alone reclaims space.
