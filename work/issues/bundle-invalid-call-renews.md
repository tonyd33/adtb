---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 895
summary: a registered call refused for its argument count or principal renews the bundle lease before validation
branch: registered-admission
tags:
  - quality
  - engine
---

# A call refused before execution still renews its bundle lease

## Report

At `c317ef9a`, `Registered::entry` (`crates/adtb/src/api/registered.rs:725-743`) renews the bundle before returning the entry. `Registered::call` then validates the principal at line 683 and the argument count through `prepared` at lines 684 and 786-792. Either can refuse the call after the renewal has committed. An interactive call similarly renews at line 766 before argument and read-only checks at lines 767-769.

Deterministic simulator reproduction: register a bundle with `lease 1d` and a zero-argument query `count`, bind its handle, capture `catalog::bundles.renewed`, advance the simulated clock four hours (past the one-eighth renewal threshold), and call that existing ordinal with `[Value::Int(1)]`. Read `renewed` again without rebinding the bundle.

Actual:

```
refused call: `count` takes 0 arguments, not 1
lease before: Instant(1767225600136103); after: Instant(1767240000140768)
```

Expected: the same `Invalid` refusal and an unchanged lease. `docs/design/server.md:83-89` now explicitly promises that a refused bind or call writes no renewal. Invalid requests can otherwise keep a bundle live and keep its reader-held restrictions in effect.

Fix: perform the call's admission checks before triggering renewal, including principal, argument and transaction-mode validation. Add regressions for wrong arity, a refused principal and a write callable in a read-only transaction. Preserve successful-call renewal and the existing untranslatable-call regression.

Related [bundle-bind-lock-across-commit.md](bundle-bind-lock-across-commit.md) records the earlier renewal-before-`Untranslatable` case as fixed and leaves nested-transaction renewal and failing-renewal coverage open. This is the remaining pre-execution refusal path, not the lock or nested-transaction issue. Standalone reproduction: `/tmp/opencode/quality27-refused-renewal.rs` in the review session; the history above is sufficient to recreate it.

## 2026-10-28: shared registered admission track

Fresh registered-admission workspace on39dc440e owns this alongside
registered-argument-values. Validate all pre-execution admission conditions before
lease renewal, using the existing shared value validator for argument values.
Retain exact renewed timestamps and original failing witnesses; positive renewal,
earlier-pin refusal and interactive transaction usability remain controls. No
separate lease-lock redesign or transport workaround. Acceptance and evidence
requirements are also in registered-argument-values. Broad roles work only audits
in parallel and does not write shared signatures.
