---
title: How a union-index uniqueness check sees concurrent writers
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  wait-inserter:
    title: Fetch the entry's tuple through its constructor's heap, wait on an in-progress writer
    links:
      - {rel: prior-art, to: prior/engine-pg-unique}
  key-lock:
    title: Lock the key value itself in a lock table
  commit-validate:
    title: Record the key as read, validate at commit
    links:
      - {rel: prior-art, to: prior/engine-fdb}
  index-visibility:
    title: Entries carry creating and deleting transaction, checked without a heap fetch
---
`insert identities.Google { id: 7, ... }` must fail if an `EmailPass` row
with id 7 is committed or being inserted concurrently. Under MVCC the
check cannot use the inserter's snapshot: a row committed after it
began still conflicts.

## engine/wait-inserter

Postgres `_bt_check_unique` with the heap chosen per entry: equal-key
entry found, tuple fetched from the heap the constructor id names,
checked against the latest committed state; if its writer is in
progress, wait and recheck. Entries for dead versions are skipped, so a
key may hold several entries, at most one live.

## engine/key-lock

The inserter locks `(index, key)` before probing, as InnoDB insert
intention and next-key locks do. Independent of which heap holds the
row.

## engine/commit-validate

The probe is a read of key range `[7, 7]` on the union index; a
concurrent insert of the key writes that range, so one of the two fails
validation at commit. Uniqueness under OCC falls out of read tracking.

## engine/index-visibility

Each entry stores its version's begin and end transaction, as
Hekaton-style version records do (unverified for disk B-trees; MV-PBT,
Riegger et al., is a multi-version B-tree precedent, unverified). The
check reads only the index; heaps are touched only to return rows.
