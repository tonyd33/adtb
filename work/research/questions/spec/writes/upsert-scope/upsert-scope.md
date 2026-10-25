---
title: "Which uniques an upsert may test"
pick: one
options:
  subset:
    title: "The target's constructor set is a subset of the unique's; an expression unique conflicts on equal expression values"
  any-overlap:
    title: "Any unique of the target relation variable"
---
Raised by the implementability review of docs/spec/. writes §6.1 let an
upsert name any declared unique, including one whose constructor set
does not hold the insert's target constructor, where no conflict can be
defined.

## spec/writes/upsert-scope/subset

The upsert's target constructor set must be a subset of the unique's
constructor set; otherwise a type error. For a unique over expressions,
a conflict is equality of the unique's expressions evaluated on the two
rows.

```adt
insert identities.EmailPass { user: u, email: e, hash: h }
unless conflict on (lower(email)) in EmailPass
else update old set old
```

## spec/writes/upsert-scope/any-overlap

Any unique of the relation variable. A row outside the unique's scope
never conflicts, so the else branch is dead code with no diagnosis.
