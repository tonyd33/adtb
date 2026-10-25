---
title: "What a migration denotes as a write"
pick: one
options:
  one-statement:
    title: "One statement: the composite of its operators' forward maps; constraints of the final schema checked once"
  per-operator:
    title: "One statement per operator, each checked against the constraints of the schema it produces"
---
Raised by the implementability review of the spec in docs/spec/. The
spec said both that a migration's script is checked at the end of the
migration and that each operator's data map is its own statement with
the constraints of its schema checked on its result. The two disagree
when a script repairs data that a constraint added earlier in the same
migration needs.

## spec/evolution/migration-statement/one-statement

A migration is one statement of its transaction. It denotes the
composite of its operators' forward maps in order. A script's forward
map applies its change set, evaluated on the state its preceding
operators produce, at their schema. Every constraint of the final
schema holds after the migration, checked once; an added constraint is
validated on the final state:

```adt
-- 0013: the unique holds once the script has run
module auth
add unique (gid) in identities.Google
script
  delete i in identities.Google where i.gid == ""
```

## spec/evolution/migration-statement/per-operator

Each operator's data map is a statement, and the constraints of the
schema it produces are checked on its result. The example above is
refused at the `add unique` whenever two rows have an empty `gid`, so
the repair must come in an earlier migration, and the operator order
inside a migration changes which migrations apply.
