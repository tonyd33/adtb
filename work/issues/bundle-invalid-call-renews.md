---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 895
summary: a registered call refused for its argument count or principal renews the bundle lease before validation
closed: 2026-10-28
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

## 2026-10-28: broader preflight ordering audit

Both original baseline cases are reproduced on ca493bf0 (exit100); details and
retained evidence are in registered-argument-values. Lead found stop/deadline,
after-position and interactive ended-state/open checks downstream of renewal.
Assess those pre-execution refusal paths with existing owning boundary helpers;
preserve one engine-open guard rather than introducing nested RwLock reads or
a pager-close workaround. A possible closed-database interactive renewal is
source-inferred, not yet an executed crash witness. Preserve exact errors and
transaction usability, and distinguish late execution failures from admission.

## 2026-10-28: additional witnesses and narrow owning hook

Retained readiness baseline reproduced renewal on initially cancelled calls,
past standalone deadlines, invalid after-positions, cancelled interactive calls
and expired transaction deadlines. A corrected ended-state witness returned the
original Serialization error but changed renewed from00:00:00.135854 to
04:00:00.149982 (exit100). Its earlier resultless-update fixture assertion failure
was setup failure, not an engine diagnosis; both logs remain retained.

The closed-database interactive control already PASSED before the hook change.
No closed-pager crash was reproduced or claimed fixed. It remains a regression
control while the code preserves the owning engine guard.

Frozen candidate a5b883281e07474bc8373dc345d2b9c20deabb09 uses a narrow FnOnce
renewal hook in existing Engine::call and AdHocTransaction::run, after their
native option/bookmark/state/merged-stop checks and before execution. No new
enter guard or duplicated readiness policy. Ordinary ad hoc calls supply a
no-op; this hook is not a per-retry authorization check. All173affected public
tests,4shared-validator unit tests and strict lint/fmt/house/size passed on that
source; full branch and fresh independent review remain in progress. Do not
interpret this as a new promise about cancellation or failures after admission.

## 2026-10-28: resolved

Landed `744bddb0`, fresh reviewed/frozen a5b88328. Registered admission validates
ordinal, lifecycle/current pin, principal, argument count/representation and
read-only mode before renewal eligibility. Existing Engine::call and
AdHocTransaction::run invoke a narrow FnOnce renewal hook only after their native
stop/bookmark/state/merged-deadline checks, under the existing engine guard.
Ordinary ad hoc calls supply a no-op. No duplicate policy, extra enter guard,
lease-lock redesign or future per-retry authorization promise was introduced.

Twelve added public tests cover invalid/valid typed values and exact renewal
timestamps across standalone, Arrow and interactive calls. Refused interactive
calls preserve prior pending work/usability; ended transactions retain their
Serialization error. Closed-database control passed even before the fix and
remains a guard, not evidence of a repaired pager panic. Late cancellation and
execution failures remain distinct from initial admission refusal.

Full branch and main checks passed: 2,426 root tests,47campaign,allstatic/docs.
Main clean4b5eec71/parent744bddb0 collected0 in250s, all inputs unchanged.
Exact command/revisions/baselines/receipt/hash are in registered-argument-values
and tracked work/briefs/*registered-admission*.md. Related lease locking and
nested-transaction coverage in bundle-bind-lock-across-commit remain open;
this resolution covers the pre-execution admission ordering described here.

## 2026-10-28: resolved
