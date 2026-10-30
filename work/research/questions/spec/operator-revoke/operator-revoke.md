---
title: "As which role an operator revoke acts"
pick: one
options:
  schema-owner:
    title: "As the schema owner, the grantor of every operator grant"
  applier:
    title: "As the role that applies the migration"
---
Raised by the spec-rev30 review round 2 (R1). With the schema owner as grantor of
every operator grant and a revoke deleting only the issuer's rows, a `schema` holder
other than the owner could revoke nothing, and replay names no revoking role.

## spec/operator-revoke/schema-owner

An operator revoke acts as the schema owner. A role holding `schema` revokes any
operator grant through a migration, and a log replay needs no revoking role.

## spec/operator-revoke/applier

Only the owner could revoke operator grants, and replay could not decide.
