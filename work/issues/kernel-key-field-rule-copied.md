---
created-at: 2026-11-15
status: open
kind: bug
priority: 600
summary: "Build::field_ty and the validator's field each inline the key-field narrowing rule kernel.md states through Schema::self_key, so the two copies and the helper can drift apart"
tags:
  - quality
  - lang
  - class-copied-rules
---

# The kernel's key-field typing rule is written out twice beside the helper the contract names

## Report

Found by the quality review of reviewed..582d7af5. kernel.md §3 states the `Field(e, l)` row through `Schema::self_key(r, l)`: a key field `l` of `r` read from `r`'s row type at `S` is typed `Key r.S`. The builder (`Build::field_ty`, crates/adtb-lang/src/kernel_build.rs:306) and the validator (`Validator::field`, crates/adtb-lang/src/kernel_valid.rs:751) each inline the same predicate instead: the field's declared type is `Key r` with no constructor set, `r`'s element type is the row's `def`, and `r`'s key names `l`. This is not `self_key`, which starts from the relation and checks the field's type is a key of that relation; the two can differ where two relations share an element type. A change to one copy makes the builder build what the validator refuses (a panic under debug assertions) or the reverse.

`Validator::row` and `Build::row` (kernel_valid.rs:231, kernel_build.rs:295) likewise compute a relation's row type at a set twice, the builder normalising the set and the validator not.

Fix direction: one function in the kernel (or `check::ty`) returning a field's type read from a row type, used by both, written through `Schema::self_key` as the contract states; the same for the row type at a set.
