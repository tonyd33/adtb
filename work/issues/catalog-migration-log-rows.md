---
created-at: 2026-10-25
status: open
kind: feature
priority: 706
summary: "no migrations, migration_ops, migration_clauses or functions rows, and no version hash"
depends-on:
  - canonical-term-form
---

# Feature: migration log rows and the version hash

## Report

Write `catalog::migrations`, `migration_ops`, `migration_clauses` and
`functions` rows for the first migration, with BLAKE3-256 hashes: the
chained version hash over the encoded operators and clauses, and a
function's hash over its body's canonical form. Encode `Operator` and
`ClauseBody` values with the wire value encoding, tags being their
fixed catalog constructor ids. Add `blake3` to `deny.toml`.
