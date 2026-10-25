---
title: Commit-time at-least-one checks race under snapshot reads
pick: any
raised-by: [constraint/at-least-one]
options:
  by-isolation:
    title: Checks are reads of a serializable transaction
    links:
      - {rel: prior-art, to: prior/engine-pg-ssi}
  parent-lock:
    title: Writes to the relationship lock the parent key
    links:
      - {rel: prior-art, to: prior/engine-oracle-assertions}
  materialized-count:
    title: The engine keeps a per-parent count both writers update
  serial-commit:
    title: Checks run in a serialized commit step against the latest state
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
  live:
    title: Live with concurrent deletes leaving a parent with none
    excludes: [engine/write-skew/by-isolation, engine/write-skew/parent-lock, engine/write-skew/materialized-count, engine/write-skew/serial-commit]
    assessment: {grade: broken, why: the engine claims a constraint it does not enforce}
---
`constraint has_identity = from u in users where count(i for i in identities where i.user == key u) == 0 select u`. User 3 has two
identities. T1 deletes one, T2 the other; each check at commit sees the
other's identity in its snapshot; both commit; user 3 has none. Classic
write skew. Exclusivity and totality are structural (a row is in exactly
one heap) and cannot skew.

```
-- T1                                   -- T2
delete i in identities where i.id == 7  delete i in identities where i.id == 8
commit  -- sees 8                       commit  -- sees 7
```

## engine/write-skew/by-isolation

Under SSI or OCC the check's read of `identities` where `i.user == 3`
conflicts with the other's delete; one aborts. If weaker levels exist,
checks must still run with serializable read tracking, or weak
transactions may not touch constrained relations.

## engine/write-skew/parent-lock

Each insert, delete or retag in a constrained relationship takes a lock
on user 3, materializing the conflict (Fekete et al., TODS 2005,
unverified). Oracle assertions lock join column values the same way.

## engine/write-skew/materialized-count

User 3 carries a hidden count of identities; deletes decrement it and
write-conflict on the row. The check is `count > 0`.

## engine/write-skew/serial-commit

Commits pass through one critical section that runs deferred checks
against the newest committed state, with a predicate read of the
relationship.

## engine/write-skew/live

Applications serialize themselves.
