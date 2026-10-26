---
created-at: 2026-10-25
status: open
kind: feature
priority: 708
summary: "six choices in the migration log rows are unstated in the spec"
---

# Feature: state the migration log choices

## Report

From the migration log track. My rulings; state each in `docs/spec/catalog.md` through the design space:
1. An unscoped unique or index has no `Member` clauses; it ranges over every constructor, including ones added later. Only a written `in r.C` scope gives `Member` clauses.
2. The `body` Term of `AddConstraint` and `AddCheck` is the whole declaration after the verb, so a check keeps its `on` target.
3. The `fields` Term is the field list without its parentheses.
4. `functions.signature` and `body` are as written; an aggregate's body is everything after `=`.
5. A language function is `deterministic` only when every function it calls, transitively, is. The code sets true today; change it.
6. `migrations.applied` is the wall-clock time the migration was applied in its transaction.
