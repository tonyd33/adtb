---
created-at: 2026-11-14
status: resolved
closed: 2026-11-16
kind: bug
priority: 430
summary: "pending ahead relations are keyed by name, so a relation added then renamed in one migration stays pending under its old name until the migration ends"
tags:
  - class-identity-by-spelling
---

# Ahead relations are keyed by name

## Report

`Source::ahead` and `schema::Ahead` (crates/adtb-lang/src/evolve, schema) key the relations a later operator adds by name. After `add relation stamps` then `rename relation stamps to labels` in one migration, `stamps` reappears in `schema.ahead` until the migration ends, so no later source in that migration settles. Harmless to log resolution since the log-resolves track keeps the names from just after each operator; any reader of `ahead` that compares names can be misled. Found in the log-resolves track; not reproduced.

Direction: key a pending relation by the id it will take (named ids are given per operator since step 6a-2), and drop it from `ahead` when the operator that adds it applies, whatever it is called later.

Defect class: Catalog rows and identity by position or spelling.

## Resolved

Landed through committed-schema-type parts 4a-1 (ystwnzvs 7e7cf707) and 4a-2 (wuztvzvl df7fafa3, 2 rounds each, full check 0, 3837 tests). A pending relation is linked to the operator that adds it, by its position in its migration (`evolve::Listed`, `Adder::Operator(k)`), and a relation a revert restores to its inverse step (`Inverse::RestoreRelation`, `Adder::Inverse(s, j)`); pending-ness and late selections go by that link. The three readers that still compare a name resolve operator or source text (`relation_ahead`, `expecting`'s and the restore's `typed`). Tests: evolve_tests add-then-rename, evolve_revert_tests restore-then-rename and restore typed by its declaration, scripts oplog/relation-added-dropped-added-again.test and relation-renamed-away-then-added-again.test.
