---
title: A foreign key to the sum has no single table to reference
pick: any
raised-by: [[encoding/variants-only, fk/to-sum]]
options:
  trigger:
    title: Trigger-emulated FK checking the union
    assessment: {grade: costly, why: "triggers on referrer and every constructor table, locking"}
  union-fk:
    title: Native FK targeting a union of relations
    excludes: [substrate/pg-compiler]
    links:
      - {rel: variant-of, to: encoding/exclusive/union-index}
  claim-table:
    title: Claim table holding every id, FKs target it
    links:
      - {rel: same-as, to: encoding/tag-variants}
  live:
    title: Live without FKs to the sum
    excludes: [encoding/sum-fk/trigger, encoding/sum-fk/union-fk, encoding/sum-fk/claim-table]
---
`sessions.identity_id REFERENCES identities` cannot be declared when
`identities` is a view over `google` and `email_pass`.

## encoding/sum-fk/trigger

`sessions` gets an insert trigger checking the view; each constructor
table gets a delete trigger checking `sessions`.

## encoding/sum-fk/union-fk

```
FOREIGN KEY (identity_id) REFERENCES (google.id ⊎ email_pass.id)
```

Rests on the same union index as `encoding/exclusive/union-index`.

## encoding/sum-fk/claim-table

`identity_keys(id PK, kind)`; each constructor references `(id, kind)`.
An FK can then reference a claim with no constructor row, so totality is
lost again for referrers. This is `encoding/tag-variants` with its
totality gap.

## encoding/sum-fk/live

Referrers point at a specific constructor or carry no FK.
