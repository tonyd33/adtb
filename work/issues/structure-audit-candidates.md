---
created-at: 2026-11-07
status: open
kind: bug
priority: 20
summary: unreproduced bugs the 2026-11-07 structural audits found by reading; each is reproduced or dismissed inside the restructure that owns its code
tags:
  - class-none
---

# Likely bugs the structural audit found by reading

## Report

## 2026-11-07

Found by reading in the structural audits (reports in work/briefs/audit-structure-r1-*.md, local); none run. Each is reproduced or dismissed by the restructure that owns its code.

- api/verify.rs:459-462: `?` after `begin` returns without aborting the transaction (structure-transactions).
- api/statements.rs:113-114: `ends_transaction` must be read before `quarantining`; swapped, a transaction that met damage stays open (structure-damage-policy).
- heap/handle.rs:318: `Heap::raw` asserts the page owner before the version check, so a page vacuum freed and another heap reused may panic where `Changed` is due (structure-version-owner).
- heap/handle.rs:998: `copy_version` leaves the source page out of the written pages; two paths skip clearing all-visible (structure-version-owner).
- catalog/mod.rs:1058: `naming_sequences` can overwrite a `Changing` marker the open kept at catalog/mod.rs:699 (structure-version-owner).
- evolve/ids.rs:217-240: `rename_field_ids` ignores the constructor, so renaming `T.B.x` gives an index `(x) in A` a new id; confirmed by reading (structure-lang-identity).
- revert of `drop type` loses `value_states`, so a declared value constructor returns Enabled (revert/inverse.rs:31-38,166-171; revert/mod.rs:1204-1206) (structure-lang-identity).
- `check_query_in` and `check_txn_in` never copy `fn_rows`: `key o` over a view-returning function likely refused in a registered query or transaction but accepted ad hoc (structure-lang-identity).
- check/expr.rs:992: `Deref.reference` decided by syntax; a deref of a fabricated key may be classified total (structure-lang-identity).
- adtb-reference/src/unicode.rs:51-57 panics when `Regex::new` fails on a pattern the checker accepted; the engine returns `Err` (structure-lang-identity).
