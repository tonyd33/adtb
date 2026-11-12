---
created-at: 2026-11-12
status: open
kind: bug
priority: 340
summary: "a unique or index scoped to every constructor of its type is read as unscoped, so split, merge and nest leave the replaced name and the migration is refused"
tags:
  - class-identity-by-spelling
---

# A scope listing every constructor is read as no scope

## Report

`scoped_clauses` treats a scope listing every constructor of the type, such as `in (A | B)` over `A | B`, as no scope. Split, merge and nest then skip the clause and leave the replaced name in its text, and the migration is refused with "`A` is not a constructor of `T`". Pre-existing on main; pinned by `an_operator_on_a_constructor_of_a_scope_listing_every_constructor_is_refused_...` in evolve_ids_tests.rs, which asserts today's refusal.

Direction: keep the written scope as written (the clause names constructors) and re-scope it by the one rule; flip the pin test to the spec's outcome. Fits structure-lang-identity step 3b.

Defect class: Catalog rows and identity by position or spelling.
