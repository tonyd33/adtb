---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 706
summary: "no migrations, migration_ops, migration_clauses or functions rows, and no version hash"
closed: 2026-10-25
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

## 2026-10-25: resolved

## 2026-10-25: resolution

The first migration writes `migrations`, `migration_ops`, `migration_clauses` and `functions` rows with the chained BLAKE3-256 version hash. `Source::resolved` covers the 10 declaring operators. Apache-2.0 and BSD-2-Clause join the licence allow list for blake3, constant_time_eq and arrayref.
