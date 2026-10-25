---
title: Retag races a concurrent insert of a constructor-typed referrer
pick: any
raised-by: [fk/retag/recheck]
options:
  ctor-in-key:
    title: The constructor is part of the referenced key for locking
  by-isolation:
    title: Serializable read tracking catches the race
    needs-one-of: [engine/ssi, engine/occ, engine/s2pl, engine/single-writer, engine/deterministic]
  live:
    title: Live with ill-typed references after concurrent retag
    excludes: [engine/retag-fk/ctor-in-key, engine/retag-fk/by-isolation]
    assessment: {grade: broken, why: a to-ctor FK ends up pointing at another constructor}
---
T1 inserts `PasswordReset { identity: 7 }` with an FK to
`identities.EmailPass`; T2 retags 7 to `Google` and re-checks referrers.
Under snapshot reads neither sees the other's write.

## engine/retag-fk/ctor-in-key

The FK check takes a key-share lock on the target tuple, as Postgres
`FOR KEY SHARE`; retag counts as a key update and conflicts with it,
whereas a field update does not. FKs to the whole sum need no such
conflict, so retag takes the stronger lock only when to-ctor FKs exist
on the old constructor.

## engine/retag-fk/by-isolation

The re-check is a read of referrers; T1's insert writes that range.
Holds only for transactions at a serializable level.

## engine/retag-fk/live

No protection.
