---
created-at: 2026-11-13
status: open
kind: bug
priority: 340
summary: "a relation over a prelude record instantiated at a key type, such as Span (Key xs), panics id assignment on its implied index"
tags:
  - lang
  - quality
  - class-failure-flattened
---

# A relation over `Span (Key xs)` panics id assignment

## Report

`Schema::reference_fields` substitutes a relation's element arguments, so in `relation ps : Span (Key xs) key start` the prelude field `Span.end` is a `Key xs` reference field with an implied index. The objects walk pushes each implied index with `Named::Field(Slot::Field(i.field_ref()))` (crates/adtb-lang/src/ids/objects.rs:334) and no user-module filter. `element` and `reach` in the same file do filter out engine fields. A prelude field has no slot id in `assign`, and the expect at objects.rs:129 panics: "an object a key names takes its id in an earlier round or before it".

Repro (reproduced as a temporary test in evolve_ids_tests.rs):

```
module auth
type X = { id: Key xs }
relation xs : X key id generate sequence
```

then the migration `module auth` / `add relation ps : Span (Key xs) key start`. Expected: refused, or applied with ids. Actual: the process panics at crates/adtb-lang/src/ids/objects.rs:129. `Source::new` over the three declarations panics the same way.

Fix direction: decide whether a relation head may instantiate a prelude record at a key type. If it may, key the implied index of a prelude field by what it reaches, through `element`. If not, refuse it in the build.

Defect class: class-failure-flattened
