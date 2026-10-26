---
created-at: 2026-10-26
status: open
kind: feature
priority: 793
summary: "the language's implied-index rule and the write path's reference fields each decide which fields are references, by different tests"
tags:
  - quality
---

# Feature: one definition of a reference field

## Report

Two functions decide which fields of a relation's sum are references:

- `Schema::is_reference` (crates/adtb-lang/src/schema.rs:673), used by `implied_indexes` (schema.rs:649) to imply an index per reference field. It unwraps an `Option`, then excludes a key of the relation itself whose field is in the key.
- `refs` (crates/adtb/src/write/schema.rs:117-162), which builds the `RefField`s the write path checks and cascades over. It excludes a key field only when `self_key` (crates/adtb-lang/src/textual/form.rs:54, a schema predicate living in the printer) finds constructor 0's field typed as the relation's own key, and matches the `Option` and key shapes again by hand. It then finds the implied index by searching `IndexDesc`s for one whose first field has the same name and covers the holding constructors (schema.rs:144-150), although `Ids::implied` already names that index.

When the two tests disagree, a reference field either has no implied index, which `referring` (crates/adtb/src/write/eval.rs) reports as an internal error at the first delete, or an index nothing reads. A rename of a field to match another index's leading field could also pick the wrong index.

Fix: give adtb-lang one pub function returning each reference field of a relation with its holding constructors and whether it is optional; build `implied_indexes` and `refs` from it, take the index id from the catalog's implied ids rather than a name search, and move `self_key` out of `textual`.
