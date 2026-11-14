---
created-at: 2026-11-13
status: resolved
closed: 2026-11-14
kind: bug
priority: 330
summary: "a revert accepts a fill no inverse uses and matches fills to fields by unqualified path text; the unused fill makes the log unresolvable, so every later revert of an earlier migration is refused"
tags:
  - lang
  - quality
  - class-identity-by-spelling
---

# An unused revert fill is accepted and then blocks every earlier revert

## Report

`Cx::fill_for` (crates/adtb-lang/src/evolve/revert/mod.rs:481) picks the fill for an inverse by comparing `path_text(p) == path`, the fill's written path against the inverse's path text. `path_text` (crates/adtb-lang/src/evolve/field.rs:863) leaves out the module. Nothing refuses a fill clause that no inverse consumes. The fill paths are already tied to field ids (crates/adtb-lang/src/evolve/operands.rs:351), but matching does not use them.

Later, `Source::resolved` resolves each fill path of a logged `revert` with `?` (crates/adtb-lang/src/evolve/resolved.rs:998). A fill naming no field makes that migration fail to resolve. The dependency gate then refuses every revert of an earlier migration with "the log does not resolve". The log is append-only, so the author cannot repair it.

Repro (reproduced as a temporary test in evolve_revert_tests.rs over its `BASE`):

```
module auth
add constructor T.C { z: Text }          -- 0001
module auth
drop field T.A.x                         -- 0002
module auth
revert 0002
  fill T.A.x = "q"
  fill T.B.typo = "q"                    -- 0003, applies
module auth
revert 0001
```

Expected: `revert 0001` applies, as it does when 0003 omits the `T.B.typo` fill. Actual: refused, `the log does not resolve: an object this operator creates or names is gone by the migration's end`.

The same matching by text, found by reading and not reproduced: when one migration drops `T.f` of the migration's module and `b::T.f` of module `b`, a revert with `fill T.f = 1` and `fill b::T.f = 2` can give both fields the first fill whose text is `T.f`.

Fix direction: match each inverse that needs a fill to its fill clause by the field's catalog id, which `tie_operands` already gives. Refuse a fill that names no field, or that no inverse of the reverted migration consumes, when the revert is applied.

Defect class: class-identity-by-spelling

## Resolved

Landed as tsvtkzlo b5496637 (4 rounds, full check 0, 3756 tests). `Source::resolved` resolves each logged operator against the sources just before and after it (runs of unbuilt sources against the nearest built ones); `renamed_later` is gone, so add then rename of a type, relation, constructor or field in one migration applies and resolves. A revert's fills match the inverse that needs them by (owner catalog id, field name), and its logged fill clauses resolve to the field an inverse took, never by path text. Text the log does not yet hold (`Source::accept`, `Catalog::migrate`, spec examples) refuses a fill no inverse takes ("no inverse of migration NNNN needs `fill PATH`") and a second fill of one field ("the revert fills `PATH` more than once"); replay, reopen and restore never do (`Catalog::replay`, catalog_tests `a_replayed_log_folds_a_revert_with_a_fill_no_inverse_takes`). evolution.md §5.4 lists the refusal. The move-down fill hint shows the `match` the fill needs over the constructors as they stand. The law search asserts every applied migration resolves with operands held around their operator; set-scoped uniques and indexes are `KNOWN` rows (log-resolver-set-scope-as-ctor). Unresolved operators now read "an object `OP` creates or names is in neither the source before it nor the one after it".
