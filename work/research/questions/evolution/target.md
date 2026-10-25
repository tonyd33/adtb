---
title: Whether schema operators act on types or on relation variables
pick: one
raised-by: [evolution/smo]
options:
  types:
    title: Operators act on a type and every relation variable of it changes
  relvars:
    title: Data operators act on one relation variable; type operators only where the type has none
---
`add constructor Identity.Apple` changes the type `Identity`. With two
relation variables of that type, `identities` and `archived_identities`,
each needs a new heap, and indexes and keys are declared per relation
variable.

## evolution/types

The add creates an `Apple` heap in every relation variable of
`Identity`. Each relation variable's constructor-set indexes say
whether `Apple` joins them (`surface/ctor-set/growth/declare` names the
relation variable and set).

## evolution/relvars

Relation variables own their type: changing the constructors of
`archived_identities` first gives it a new type. Types shared by
several relation variables change only by operators applied to each.
