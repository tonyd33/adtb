---
title: A subset target spans several constructor tables
pick: any
raised-by: [fk/to-subset]
options:
  nested-sum:
    title: Require the subset to be a declared intermediate sum
  tag-check:
    title: Composite FK to the parent with a CHECK on the referenced tag
  subset-index:
    title: Union index over only the named constructors' heaps
    needs: [index/ctor-set]
  trigger:
    title: Trigger-emulated FK checking the named constructors
    assessment: {grade: costly, why: triggers and locking as any emulated FK}
  live:
    title: Live with only sum and constructor targets
    excludes: [fk/subset/nested-sum, fk/subset/tag-check, fk/subset/trigger, fk/subset/subset-index]
---
`identities.(Google | Facebook)` is neither one table nor the whole sum.

## fk/subset/nested-sum

```
type Provider = Password (EmailPass) | OAuth (Google | Facebook)
```

The subset becomes a constructor whose payload is itself a sum, so a
subset FK is a constructor FK one level down. Only subsets the schema
author anticipated are targets. The Third Manifesto's union types are
this shape.

## fk/subset/tag-check

```sql
oauth_tokens (identity_id bigint, identity_kind text NOT NULL
              CHECK (identity_kind IN ('google','facebook')),
  FOREIGN KEY (identity_id, identity_kind) REFERENCES identities (id, kind))
```

Works with a stored tag; the referrer carries a copy of the tag. A new
engine can make that copy a hidden virtual column. Postgres partitioned
tables put the partition key in every unique key the same way.

## fk/subset/subset-index

```
relation identities : Identity
  key id
  index (id) in (Google | Facebook)
```

Any subset is a target at the cost of one index each. Generalizes
`nested-sum` to subsets the schema does not name.

## fk/subset/trigger

Insert trigger on the referrer, delete triggers on each named
constructor table.

## fk/subset/live

Referrers name the sum or one constructor only.
