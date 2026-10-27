---
created-at: 2026-10-27
status: open
kind: bug
priority: 897
summary: "bundle tests accept any error for a refused supersede and any explain outcome without 'not registered'"
tags:
  - quality
  - test
---

# Bundle tests accept any error for a refused supersede and for explain

## Report

- `supersede_bundle_by_a_bundle_of_another_family_or_one_not_live_is_refused` (`crates/adtb-sim/tests/bundles.rs:1006`) asserts only `is_err()`. It should expect `Invalid` for another family and for a bundle superseding itself, and `UnknownBundle` for `web-9.9`.
- `explain_of_a_registered_call_checks_against_the_registered_bundle` (`bundles.rs:1057`) accepts any error for the known call as long as its text lacks "not registered".

Fix: assert the reason of each refusal, and the explain output for the known call.
