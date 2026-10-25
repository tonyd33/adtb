---
title: A retag moves a row into scanned constructor relations without inserting a new key
pick: any
raised-by: [[engine/ssi, union-index/retag/move]]
options:
  as-insert:
    title: The retag's index writes are checked against range locks as inserts of the new constructor
  live:
    title: Live with retags invisible to range locks
    excludes: [mvcc/retag-phantom/as-insert]
    assessment: {grade: broken, why: a serializable scan of Google misses a row retagged into Google}
---
Postgres SSI: an update that moves a row into a scanned range must
conflict (https://raw.githubusercontent.com/postgres/postgres/master/src/backend/storage/lmgr/README-SSI). A retag keeps the key, so the union index entry
for `(7)` is rewritten from `EmailPass` to `Google` in place or beside
the old one, and no new key appears in the range. A scan of
`identities.Google` by key read 7's slot and found nothing visible.

```
T1: count(i for i in identities.Google where i.id >= 1 and i.id <= 10)
T2: update i in identities.EmailPass where i.id == 7 set Google { gid: g }
```

## mvcc/retag-phantom/as-insert

Each union index entry the retag writes, on every index whose
constructor set contains `Google`, is checked against range locks as
an insert of `(7, Google)`: with `mvcc/whole-range` any lock covering
7 conflicts, with `mvcc/ctor-tagged` only those whose set contains
`Google`. A scan that filtered `EmailPass` entries in the index took no
tuple lock on the old version, so the tuple check alone misses it.
Tuple locks on the old version conflict as for any update. Its SIREAD
locks need not follow the row to the new heap: Postgres proves later
versions need no copy (README-SSI).

## mvcc/retag-phantom/live

No check.
