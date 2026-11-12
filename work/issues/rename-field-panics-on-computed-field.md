---
created-at: 2026-11-12
status: open
kind: bug
priority: 420
summary: "rename field panics in Source::edit when a computed field names the renamed field"
tags:
  - class-identity-by-spelling
---

# rename field panics on a computed field naming it

## Report

`add computed ts.width: I64 = scalar_len(self.n)`, then `rename field T.n to n9`: panics in `Cx::rename_field` via `Cx::apply_edits` in `Source::edit` with "range end index 225 out of range for slice of length 37". An operator on author input must refuse or apply, never panic. Found writing law-search cases in structure-lang-identity step 5c-2, whose case `drop computed ts.width [named object renamed]` renames `ts` instead until this is fixed. Same on main.

Related, same cause family: on the law-search base, `rename field T.A.z to z9` is refused "no `z9` in scope" because the rename rewrites `index (z) in S`, which names the fields `z` of both `A` and `B`, by spelling.

Direction: the rename edits name spans tied to the field's id (`Logged::spelling()`, structure-lang-identity 5c-2), with offsets from the text it edits. Test: the case above in the law search, and the same with the computed field.

Defect class: Catalog rows and identity by position or spelling.
