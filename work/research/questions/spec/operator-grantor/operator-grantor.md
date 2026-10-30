---
title: "Who is the grantor of an operator grant"
pick: one
options:
  schema-owner:
    title: "The schema owner"
  applier:
    title: "The role that applied the migration"
---
Raised by the spec-rev30 review round 2 (G7). Revoke deletes the rows the issuer
granted, so the grantor of a log grant decides who may revoke it.

## spec/operator-grantor/schema-owner

Every operator grant has the schema owner as grantor, whichever role holding
`schema` applied the migration.

## spec/operator-grantor/applier

The role that applied the migration is the grantor, so another holder of `schema`
could not revoke it.
