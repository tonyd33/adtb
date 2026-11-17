---
created-at: 2026-11-07
status: resolved
closed: 2026-11-16
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
- catalog/mod.rs:1058: `naming_sequences` can overwrite a `Changing` marker the open kept at catalog/mod.rs:699 (structure-version-owner). Reproduced and fixed 2026-11-16 by catalog-roots-changing (cadb3d57): the root page marker has one owner in catalog/root.rs, and naming the counters joins a kept marker.
- evolve/ids.rs:217-240: `rename_field_ids` ignores the constructor, so renaming `T.B.x` gives an index `(x) in A` a new id; confirmed by reading (structure-lang-identity).
- revert of `drop type` loses `value_states`, so a declared value constructor returns Enabled (revert/inverse.rs:31-38,166-171; revert/mod.rs:1204-1206) (structure-lang-identity).
- `check_query_in` and `check_txn_in` never copy `fn_rows`: `key o` over a view-returning function likely refused in a registered query or transaction but accepted ad hoc (structure-lang-identity).
- check/expr.rs:992: `Deref.reference` decided by syntax; a deref of a fabricated key may be classified total (structure-lang-identity).
- adtb-reference/src/unicode.rs:51-57 panics when `Regex::new` fails on a pattern the checker accepted; the engine returns `Err` (structure-lang-identity).

## 2026-11-16, triaged; resolved

All owning restructures have resolved; the triage (receipt-audit-candidates-r1, local, on main 2efdbea4) probed the 9 left. Dismissed, with the commit that removed the shape: verify `?` after `begin` (6101a53c, 4248d1a5: `Txn` drops, a failed write is abandoned), ends before quarantining (ec4ff7ad: one `ends` in `Failed`), `Heap::raw` owner assert (620ef19d: `raw` returns `None`, checked to `Changed`), `copy_version` all-visible (a736dcab: `HeapWrite::commit` clears every changed page), `rename_field_ids` (8e45a75a; probe keeps id 1025), revert `drop type` value states (7d4e19cf, with its test). Live, filed: registered-check-skips-function-rows, reference-deref-total-by-syntax, reference-regex-size-panics. Also filed: copy-version-test-clears-by-delete.
