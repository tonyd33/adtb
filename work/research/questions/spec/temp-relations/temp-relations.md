---
title: "Where staging rows of a one-off load live"
pick: one
options:
  temp:
    title: "Session-scoped temp relations outside the log, keyed by their key or whole row"
  migration:
    title: "Ordinary relations created by migrations"
---
Raised by the second review of the spec in docs/spec/.

## spec/temp-relations/temp

Session-scoped temp relations outside the log, keyed by their key or whole row. A one-off load should not bump the schema version and recheck every bundle.

## spec/temp-relations/migration

Ordinary relations created by migrations. Every load becomes a schema change.
