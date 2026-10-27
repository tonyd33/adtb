---
created-at: 2026-10-27
status: open
kind: bug
priority: 898
summary: "the manual says add default and add computed are refused although both now apply, and gives the old refusal for store and script"
tags:
  - quality
  - devex
---

# The manual marks working default and computed-field migrations unavailable

## Report

At `c317ef9a`, `docs/manual/README.md:83-87` says the engine refuses `script`, `store`, `add default` and `add computed` with "this operator has no resolved form yet". `docs/manual/reference/language.md:611-615` repeats that refusal for defaults, computed fields and scripts.

Commit `6be9c898` added resolved forms and enabled engine application of defaults and computed fields before the manual commit landed. `crates/adtb-lang/src/evolve/derived.rs:13-77` resolves them, and `crates/adtb-sim/tests/oplog.rs:1398-1501` exercises their application, reads, drops and reverts. `store` is now accepted too, though physical storage and maintenance remain incomplete as [stored-views.md](stored-views.md) records. A script now fails with "the engine does not run a `script` yet" from `crates/adtb/src/catalog/mod.rs:513-520`, not the old resolved-form refusal.

Verification command from the review workspace:

```
direnv exec /home/tony/code/adtb work/tools/suite -p adtb-sim --test oplog -E 'test(an_added_default) | test(an_added_computed) | test(a_stored_view)'
```

Actual: five tests passed, including added-default application/revert, computed-field reads/catalog rows, and accepted stored-view reads. Expected: the manual distinguishes those working operations from physical stored-view maintenance and from refused scripts, and names current refusals accurately.

Fix those availability lists and add runnable migration examples for defaults and computed fields. The prose is outside the fenced examples, so the manual's passing executable examples do not catch this contradiction. [user-manual.md](user-manual.md)'s latest open work is reader sign-off, not this specific availability regression.
