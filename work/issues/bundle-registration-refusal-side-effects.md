---
created-at: 2026-10-27
status: wontfix
kind: bug
priority: 1013
summary: refused bundle registration persists headers and supersedes existing bundles
closed: 2026-10-27
---

# Bundle registration refusal side effects

## Report

A system-write guard test observed a bundle header after Database::open returned
RegistrationRefused/Denied. This was provisionally diagnosed as a violation of
registration atomicity. The proposed repair was stopped before production edits
when a fresh audit checked the specific embedded-open contract.

## 2026-10-28: wontfix, the observed behavior is required

applications.md section 8 and embedding.md section 2.1 explicitly require unchecked
bundle headers and supersession to commit before migrations. Later failure does
not undo them. Only the callable registrations of step 5 are atomic. The adopted
ruling spec/embedded-open/register-first confirms that ordering; an unchecked
header is not yet a reader. Existing bundles.rs tests cover durable supersession
through refusal and all-or-none callable registration.

The lead initially approved a general repair from the broader registration rule
without checking this embedded exception. The independent bundle-atomic audit
caught that mismatch before source edits or builds. No production change is
warranted and no design ruling has changed. The guard test must expect the early
header/supersession and check that no step-5 callable rows or lease renewal occur.
The ordinary program-write protection defect remains real and separate.

Audit workspace /home/tony/code/adtb-bundle-atomic is empty on main261ffdec.
Report and short handoff are in work/briefs; no test/build was run, house check
passed. Preserve the original failed oracle and this correction as evidence.

## 2026-10-27: wontfix
