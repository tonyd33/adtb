---
created-at: 2026-11-13
status: open
kind: bug
priority: 330
summary: "a migration fill that prints a value holding a key to a relation a later operator adds panics the engine in Form::of"
tags:
  - lang
  - engine
  - quality
  - class-failure-flattened
---

# A fill printing a key to a later relation panics the engine

## Report

A fill is compiled against the step's schema, which may hold `Key r` types whose `r` is `RelRef::Ahead`. The checker refuses a dereference, `value` or `match` through such a key by `Schema::relation_of`. `print` and `to_text` (crates/adtb-lang/src/check/prelude.rs:725) are admitted for any non-function type. Executing them builds a `Form` (crates/adtb/src/exec/prim/mod.rs:22, :449). `Form::of` (crates/adtb-lang/src/textual/form.rs:152) calls `Schema::declared`, which panics on `Ahead`: "a committed schema's key names a declared relation". c10fb37c gave `exec/store.rs` `conv` an `Ahead` case for this same fill schema, but not `Form::of`.

Repro (reproduced as a temporary engine script under crates/adtb-sim/scripts/oplog/):

```
seed 73

migrate
  module m
  add module m
  add type O = { id: Key os, n: I64 }
  add relation os : O key id generate sequence

run
  insert os { n: 1 }

migrate
  module m
  add field O.u: Option (Key later) fill None
  add field O.s: Text fill match
    | O o -> print(o.u)
  add type L = { id: Key later }
  add relation later : L key id generate sequence
```

Expected: the migration is refused with "`later` is added by a later operator of this migration", as for `value`, or it applies with `s` = `"None"`. Actual: `the engine panics: a committed schema's key names a declared relation: `later` is added by a later operator of this migration`.

`Schema::declared` has about 17 callers in adtb-lang, the engine, codegen, adtb-file and the reference. Its precondition (a schema with no migration being applied) has no enforcer.

Fix direction: refuse a type-driven primitive (`print`, `to_text`) whose argument type reaches an `Ahead` key in the checker, or give `Form::of` an `Ahead` form as `layout::key_value_ty` does. Then make the mid-migration schema a distinct type, or make `declared` unreachable on one.

Defect class: class-failure-flattened
