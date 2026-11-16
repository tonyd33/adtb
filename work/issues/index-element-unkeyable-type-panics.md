---
created-at: 2026-11-15
status: open
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
