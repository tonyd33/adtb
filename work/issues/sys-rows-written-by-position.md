---
created-at: 2026-11-16
status: open
kind: bug
priority: 150
summary: "sys:: volatile rows are written by field position and constructor 0"
tags:
  - engine
  - class-identity-by-spelling
---

# sys:: volatile rows are written by field position and constructor 0

The volatile `sys::` relations are filled with `Val::ctor(0, vec![...])`, each field placed by its position in the declaration and the constructor taken as index 0:

- `process_options` in crates/adtb/src/api/settings.rs
- `Shown::row` in crates/adtb/src/write/vacuums.rs
- the two row builders in crates/adtb/src/write/past.rs
- `phase` in crates/adtb/src/api/verify.rs

A reorder of a field in the relation's declaration in catalog.adt, or a second constructor, changes what each value means with no failure: a text lands in a text field of another name, a number in another number. The catalog relations are written through `catalog::codec` by fixed constructor id and field handle, checked against the declaration when the codec is built; these rows are not.

Found while counting catalog writers for catalog-encoders-through-codecs, left out there because these rows are never stored.

Fix direction: a codec per volatile relation in `Coded`, its writers through `Codec::encode`, so the searching `read_back` test covers them under the reversed catalogs; then a structure-check row counting `Val::ctor(0,` in production sources outside the codec.

Defect class: Catalog rows and identity by position or spelling.
