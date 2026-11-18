---
created-at: 2026-11-15
status: resolved
closed: 2026-11-17
kind: bug
priority: 150
summary: an index element of a set or unit type is admitted by the checker and panics the engine at migrate
tags:
  - engine
  - class-failure-flattened
---

# Index element unkeyable type panics

## Report

Found by structure-lang-identity step 10c-2 on cf92f157. The checker types an index element with `synth` over the row and accepts `index ([n])` and `index (())`; the migration that adds such an index panics the engine at crates/adtb/src/catalog/layout.rs:127 ("Set(Base(I64)) is not a sum", "Unit is not a sum") while laying out the key.

Expected: an index element whose type has no key form is refused when the schema is checked, naming the element and its type, as every other inadmissible index element is refused. The rule is the key-form rule of index keys (which types have an order-preserving key encoding); state it once, in the checker or in admission, and have layout rely on it.

Repro: a schema with `index ([n]) in items` over a relation with an `n: I64` field, migrated through the engine.

Defect class: Damage or failure gives a panic, hang or flattened error.

## Resolution 2026-11-17

Fixed by `fix: refuse an index element with no key form at check and a set-valued one at migrate`. Rule, in derived.md §5.2: an element of an index, unique or interval index contains no set or function type at any depth, except that an index element may be a top-level set whose element type contains none (interop.md §11.2); `()` is the empty record. One predicate, `Ty::key_form`, used by the checker and stated as `key_type`'s precondition. The engine refuses a set-valued element at migrate as unimplemented (multi-entry-index). The refusal names the element, its relation and its type. Sweep: 90 rows on the reference and the engine. Follow-ups: multi-entry-index, interval-index-element-type-unstated, refused-element-cascades-scope-error.
