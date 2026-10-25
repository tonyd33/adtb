---
title: "What value the version hash encodes for an operator"
pick: one
options:
  catalog-sum:
    title: "A catalog sum Operator, one constructor per operator, objects by catalog id; operators are rows keyed by (migration, ordinal), hashed in the wire value encoding"
  internal-form:
    title: "An internal resolved form the spec leaves unspecified"
---
Raised while applying the ruling that the version hash encodes the
resolved operators in the wire value encoding. That encoding needs a
type for the operators, and the catalog had none.

## spec/evolution/operator-type/catalog-sum

The catalog has a sum type `Operator` with one constructor per schema
modification operator, each with its operands, every object named by
its catalog id. A migration's operators are rows of
`catalog::migration_ops` keyed by `(migration, ordinal)`, and the
clauses an operator takes any number of are rows of
`catalog::migration_clauses`, since fields hold no collections. The
version hash encodes those values in the wire value encoding, in
ordinal order.

```adt
type MigrationOp = { migration: Key migrations, ordinal: I32, op: Operator }
relation migration_ops : MigrationOp key (migration, ordinal)
```

## spec/evolution/operator-type/internal-form

The hash covers an internal resolved form. Two implementations cannot
agree on a version's hash without agreeing on a form the spec does not
state, and tools cannot recompute it.
