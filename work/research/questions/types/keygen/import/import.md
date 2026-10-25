---
title: Explicitly given values collide with later generated ones
pick: any
raised-by: [types/keygen/sequence, types/keygen/snowflake]
options:
  advance:
    title: Writing an explicit value advances the generator past it
  disjoint:
    title: Explicit values must lie in a range the generator never issues
  live:
    title: Live with later inserts failing the key check
    excludes: [types/keygen/import/advance, types/keygen/import/disjoint]
---
`model/declared-key`: generated fields are settable on import. Postgres
sequences do not move on an explicit insert, so the next `nextval`
can collide.

## types/keygen/import/advance

The generator's state is `max(state, value + 1)` after the change set
commits; a non-transactional bump, as `setval`.

## types/keygen/import/disjoint

Imported ids stay negative, or carry a different node id.

## types/keygen/import/live

The duplicate fails the unique check and the client retries.
