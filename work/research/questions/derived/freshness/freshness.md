---
title: When a stored derived relation reflects a write
pick: some
raised-by: [derived/materialize/storage-choice, derived/materialize/refresh-kind]
options:
  in-txn:
    title: Maintained inside the writing transaction's change set
    links:
      - {rel: prior-art, to: prior/derived-oracle-mv}
      - {rel: same-as, to: constraint/enforce/delta}
  lagging-snapshot:
    title: Maintained after commit; a transaction reading it runs at the view's frontier
    links:
      - {rel: prior-art, to: prior/derived-materialize}
      - {rel: gains, to: engine/retry/safe-snapshot}
  eventual:
    title: Maintained after commit; readers see whatever has been applied
    assessment: {grade: broken, why: a transaction reading the view and its base is not serializable}
    links:
      - {rel: prior-art, to: prior/derived-noria}
      - {rel: harms, to: engine/serializable-only}
  on-refresh:
    title: Content is the value at the last explicit refresh
    needs: [derived/materialize/refresh-kind]
    links:
      - {rel: prior-art, to: prior/derived-pg-matview}
---
The isolation level is serializable only (`engine/serializable-only`).
A stored view read beside its base relations must be consistent with
them or the read is not serializable.

## derived/freshness/in-txn

The view's delta is computed from the transaction's change set and
added to the same multiple assignment, as constraint checks are
(`constraint/enforce/delta`). Every read at any snapshot sees the
view consistent with base data. Write cost grows with each view over
the written relations. Oracle `ON STATEMENT`/`ON COMMIT`.

## derived/freshness/lagging-snapshot

A maintainer applies committed change sets in commit order and
publishes a frontier version. A transaction that reads the view takes
its snapshot at or below the frontier, so base and view agree, at the
cost of staleness. Materialize serves strict serializable reads this
way (`kb/materialize.md`). A read-write transaction on a lagging view
reads older data than its writes conflict against.

## derived/freshness/eventual

Noria: updates reach different views at different times. Readers can
see a view ahead of or behind a base relation in the same snapshot.

## derived/freshness/on-refresh

```
refresh open_by_user   -- a write: replaces the stored relation
```

Readers see the refreshed value. Consistent at every snapshot because
the content is defined as what the last refresh wrote.
