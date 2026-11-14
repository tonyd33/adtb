---
created-at: 2026-11-14
status: open
kind: bug
priority: 230
summary: "a relation whose element type instantiates a record with a key nested inside another record is admitted, and its nested references are not enforced"
tags:
  - engine
  - class-admission-per-path
---

# A key nested in a relation's element arguments is admitted and unenforced

## Report

model.md: a reference is a direct field of a constructor or a header field of a relation's element type; no reference occurs deeper. A user type with a field `sp: Span (Key xs)` is refused at crates/adtb-lang/src/schema/build.rs:426-431, but the instantiation arguments of a relation's element type are not checked: `relation nn : Span (Span (Key ys)) key start` applies, and an engine probe inserted rows naming `ys.key(77)` to `ys.key(80)`, keys of no row, without refusal. Same on main. Found in the review of the prelude-ref-relation track; reproduced, not kept.

Direction: apply `check_references` to the substituted element type of every relation (one gate for declared and instantiated elements). Test: the relation above refused with the direct-field message.

Defect class: Authority and admission checked per entry path.
