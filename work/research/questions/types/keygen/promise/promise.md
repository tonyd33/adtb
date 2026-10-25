---
title: What a sequence promises about its values
pick: one
raised-by: [types/keygen/sequence]
options:
  distinct:
    title: Distinct values only
  session-monotone:
    title: Increasing within one session
  gapless:
    title: Contiguous values in commit order
    assessment: {grade: costly, why: every inserter conflicts on one counter and SSI aborts all but one}
---
Under SSI the serialization order is neither start nor commit order,
so "increasing" needs a reference order.

## types/keygen/promise/distinct

Postgres with `CACHE > 1`: values distinct, out of order across
sessions; Snowflake `NOORDER`.

## types/keygen/promise/session-monotone

Postgres with `CACHE 1`, which is also monotone across sessions in
allocation order (not commit order).

## types/keygen/promise/gapless

A counter row read and written by each inserting transaction; the
only way to number invoices without holes.
