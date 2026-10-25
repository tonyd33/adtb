---
title: Changing a row's constructor must re-check incoming constructor FKs
pick: any
raised-by: [fk/to-ctor]
options:
  delete-insert:
    title: Constructor change is delete plus insert, firing ON DELETE
    links:
      - {rel: same-as, to: union-index/retag/forbid}
  recheck:
    title: Retag checks every to-ctor FK pointing at the row
  live:
    title: Live without retag
    excludes: [fk/retag/delete-insert, fk/retag/recheck]
---
A `Key identities.EmailPass` in `resets` points at identity 7; identity
7 becomes `Google`. The reference is now ill-typed.

## fk/retag/delete-insert

Referrers' `ON DELETE` actions run as for a deletion.

## fk/retag/recheck

Retag fails while a to-ctor FK for the old constructor references the
row, as `RESTRICT` does.

## fk/retag/live

No retag in the language.
