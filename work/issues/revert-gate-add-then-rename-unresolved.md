---
created-at: 2026-11-13
status: resolved
closed: 2026-11-14
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

## Resolved

Landed as tsvtkzlo b5496637 (4 rounds, full check 0, 3756 tests). `Source::resolved` resolves each logged operator against the sources just before and after it (runs of unbuilt sources against the nearest built ones); `renamed_later` is gone, so add then rename of a type, relation, constructor or field in one migration applies and resolves. A revert's fills match the inverse that needs them by (owner catalog id, field name), and its logged fill clauses resolve to the field an inverse took, never by path text. Text the log does not yet hold (`Source::accept`, `Catalog::migrate`, spec examples) refuses a fill no inverse takes ("no inverse of migration NNNN needs `fill PATH`") and a second fill of one field ("the revert fills `PATH` more than once"); replay, reopen and restore never do (`Catalog::replay`, catalog_tests `a_replayed_log_folds_a_revert_with_a_fill_no_inverse_takes`). evolution.md §5.4 lists the refusal. The move-down fill hint shows the `match` the fill needs over the constructors as they stand. The law search asserts every applied migration resolves with operands held around their operator; set-scoped uniques and indexes are `KNOWN` rows (log-resolver-set-scope-as-ctor). Unresolved operators now read "an object `OP` creates or names is in neither the source before it nor the one after it".
