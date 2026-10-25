---
title: "Where staging rows of a one-off load live"
pick: one
options:
  temp:
    title: "Session-scoped temp relations outside the log, keyed by their key or whole row"
  migration:
    title: "Ordinary relations created by migrations"
  txn:
    title: "Temp relations scoped to one interactive transaction or one ad hoc text, outside the log"
---
Raised by the second review of the spec in docs/spec/.

## spec/temp-relations/temp

Session-scoped temp relations outside the log, keyed by their key or whole row. A one-off load should not bump the schema version and recheck every bundle.

## spec/temp-relations/migration

Ordinary relations created by migrations. Every load becomes a schema change.

## spec/temp-relations/txn

Temp relations scoped to one interactive transaction or one ad hoc text, outside the log. Sessions hold no state, and every connection continuing the transaction sees them.
