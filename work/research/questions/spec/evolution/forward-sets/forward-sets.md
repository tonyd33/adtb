---
title: "Whether a constructor-typed reference may name a relation added later in the migration"
pick: one
raised-by: [spec/evolution/forward-refs/later-key]
options:
  same-migration:
    title: "Key r.C and Key r.(Set) may name a relation added later when its type is added in the same migration; constructor sets resolve when the migration ends"
  whole-only:
    title: "Only Key r may name a relation added later"
---
Raised by the campaign manager. With only `Key r` seeing ahead, a sum
cannot declare a constructor-typed reference to itself in the operator
that adds its type, a cycle constructor-typed on both sides cannot be
written as `add type` operators alone, and a schema the engine prints
is not a first migration when each declaration is prefixed by its
verb.

## spec/evolution/forward-sets/same-migration

Within one migration, a reference type `Key r.C` or `Key r.(C | S)`
may name a relation variable a later operator of the same migration
adds, when the type of that relation variable is added in the same
migration. Its constructors and named sets resolve when the migration
ends, so a named set added later in the migration may be named too. A
schema printed by the engine, each declaration prefixed by its verb, is
a valid first migration.

```adt fragment
add type Entity { id: Key entities }
  = Location { parent: Option (Key entities.Location) }
  | Party
add relation entities : Entity key id generate sequence
```

## spec/evolution/forward-sets/whole-only

Only `Key r` may name a relation variable added later. A self-reference
of a new sum is added by `add field` after the relation, and a printed
schema must be reordered or split into steps to replay.
