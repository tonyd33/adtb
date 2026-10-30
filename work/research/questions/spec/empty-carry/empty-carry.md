---
title: "What happens to a grant a carry leaves with no covered field"
pick: one
options:
  drop:
    title: "The migration drops it"
  keep:
    title: "It stays as a row with no fields"
---
Raised by the spec-rev30 review round 2 (N3). `grant update identities.OAuth
(provider)`, then a split by `provider`, leaves no field carried whole.

## spec/empty-carry/drop

A `read` or `update` grant left covering no field is dropped by that migration.

## spec/empty-carry/keep

A row printing `()` cannot be written by any grant or revoke.
