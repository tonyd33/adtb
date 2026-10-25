---
title: How a retagged version names its successor in another heap
pick: one
raised-by: [[engine/in-heap, engine/retag-chain/forward-pointer]]
options:
  wide-ctid:
    title: Every header's successor link holds a constructor id beside the TID
  key-probe:
    title: A moved flag and the successor found by probing the union index with the kept key
  stub:
    title: The successor link points at an in-page redirect stub holding constructor and TID
  global-tid:
    title: The successor link holds a file-global TID; a moved flag marks a cross-heap successor
---
Retag keeps the key and moves the row to another constructor's heap.
A waiter or a chain walker must reach the new version.

## layout/succ/wide-ctid

```
ctid = u32 ctor | u32 block | u16 slot      10 bytes in every header
```

Four bytes on every tuple for an event most never see.

## layout/succ/key-probe

`flags.moved` set; the chain walker reads the dead version's key and
probes the sum's primary union index, which retag already updated to
`(key, new ctor, new tid)`. No extra bytes; one index probe per hop.

```
update i in identities.EmailPass where i.id == 7 set Google { gid: "x" }
-- old tuple: xmax = T, flags.moved; identity_pk: 7 -> (Google, tid')
-- waiter: sees moved, probes identity_pk for 7, re-checks (Google, tid')
```

## layout/succ/stub

The 6-byte ctid points at a small stub tuple on the same page (or a
line pointer flag, as Postgres redirect line pointers do within a page,
unverified) holding `(ctor, tid)`; vacuum removes the stub with the
dead version.

## layout/succ/global-tid

Page ids are global to the file, so the header's existing `u64 next`
TID names the successor in any heap of the sum. The moved flag only
says the successor is in another heap. No index probe per hop, no
extra bytes, and an A to B to A retag in one transaction leaves each
version naming exactly its successor.

```
update i in identities.EmailPass where i.id == 7 set Google { gid: "x" }
-- old tuple: xmax = T, flags.moved, next = tid' (in the Google heap)
-- waiter: sees moved, reads next, re-checks tid'
```
