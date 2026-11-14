---
created-at: 2026-11-13
status: resolved
closed: 2026-11-14
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

## Resolved

Landed as kksvmnwo 53ec1606 (4 rounds, full check 0, 3747 tests). `Form::of` gives a key of a relation a later operator adds the empty record form, as `layout::key_value_ty` does, so the migration applies with `s = "None"`. `T.parse` where `T` can hold such a key (schema/ahead.rs `holds_no_ahead_key`: direct, Option, nested record, sum field, type argument, relation key field; not tags) is refused with "`later` is added by a later operator of this migration", also in a `define fn` before the relation is added (main accepted that). `Schema::declared` keeps main's per-key panic; a whole-schema check was tried and withdrawn (it refused registrations main served). Scripts: oplog/fill-prints-key-of-later-relation.test, oplog/fill-parses-key-of-later-relation.test, three under bundles/. Left: four translate sites (map.rs:208, 344, 346; walk.rs:894) reach `declared` through a reverted fill's schema if its terms hold such a key in a form the checker admits (a match over `Option (Key zs)`, `print`); not reproduced; filed translate-reverted-fill-ahead-key; the type enforcer is committed-schema-type.
