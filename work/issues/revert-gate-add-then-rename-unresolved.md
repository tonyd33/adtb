---
created-at: 2026-11-13
status: open
kind: bug
priority: 330
summary: "a migration that adds a type or relation and renames it in the same migration does not resolve, so the revert gate refuses every revert of an earlier migration"
tags:
  - lang
  - quality
  - class-identity-by-spelling
---

# Adding and renaming an object in one migration blocks every earlier revert

## Report

`Source::resolved` resolves each operator's operands by name against the schemas before and after the whole migration. `AddType` looks up `t.name` in the schema after the migration (crates/adtb-lang/src/evolve/resolved.rs:733), and `AddRelation` does the same with `r.name` (resolved.rs:737). A rename later in the same migration moves the object to another name, so the lookup finds nothing and the operator resolves as "gone". `renamed_later` (resolved.rs:1336) follows only field renames, matched by module, type text and constructor text. It follows no rename of a type, relation or constructor.

Since the dependency gate (`Source::dependants`) refuses a revert while any later migration does not resolve, one such migration blocks the revert of every migration before it.

Repro (reproduced as a temporary test in evolve_revert_tests.rs over its `BASE`):

```
module auth
add constructor T.C { z: Text }          -- 0001
module auth
add type X = X1 | X2
rename type X to Y                       -- 0002, applies
module auth
revert 0001
```

Expected: applies, since 0002 names nothing 0001 created. Actual: refused, `the log does not resolve: an object this operator creates or names is gone by the migration's end`. Replacing 0002 with `add type V = { id: Key vs }`, `add relation vs : V key id generate sequence`, `rename relation vs to ws` gives the same refusal.

migration-rename-then-drop has the same cause, for a field renamed and then dropped under its old name. Its report covers translation, not the revert gate or added objects.

Fix direction: resolve each operator's operands against the source just before and just after that operator, as `tie_operands` does, instead of against the whole migration's endpoints. `renamed_later` then goes.

Defect class: class-identity-by-spelling
