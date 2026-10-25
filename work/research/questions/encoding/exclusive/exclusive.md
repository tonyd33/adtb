---
title: Nothing stops one id appearing in two constructor tables
pick: any
raised-by: [encoding/variants-only]
options:
  shared-seq:
    title: All constructor tables draw ids from one sequence
    assessment: {grade: hack, why: "unenforced, an explicit id breaks it"}
  trigger:
    title: Insert triggers check the other constructor tables
    assessment: {grade: costly, why: needs locking against concurrent inserts}
  union-index:
    title: Native unique index over a disjoint union of relations
    excludes: [substrate/pg-compiler]
    assessment: {grade: clean, unverified: true, why: does not exist yet}
  live:
    title: Live with possible duplicates across constructors
    excludes: [encoding/exclusive/shared-seq, encoding/exclusive/trigger, encoding/exclusive/union-index]
---
With `encoding/variants-only`, `google(id = 7)` and `email_pass(id = 7)`
can both exist, making identity 7 two constructors at once.

## encoding/exclusive/shared-seq

```sql
CREATE SEQUENCE identity_id;
CREATE TABLE google (id bigint PRIMARY KEY DEFAULT nextval('identity_id'), ...);
```

## encoding/exclusive/trigger

`BEFORE INSERT` on each table: `IF EXISTS (SELECT 1 FROM email_pass WHERE
id = NEW.id) THEN RAISE`. Two concurrent transactions both pass without
an advisory lock on the id.

## encoding/exclusive/union-index

```
CREATE UNIQUE INDEX identity_id ON (google.id ⊎ email_pass.id);
```

One B-tree whose entries point into several heaps. The primitive a new
engine (or possibly an extension) would contribute.

## encoding/exclusive/live

Accept the possibility; detect with a periodic audit query.
