---
created-at: 2026-11-14
status: resolved
closed: 2026-11-14
kind: bug
priority: 340
summary: "the ad hoc path types Group and Max outside kernel section 3"
tags:
  - lang
  - class-none
---

# the ad hoc path types Group and Max outside kernel section 3

## Report

Found by the kernel validator of structure-lang-identity step 8a (a1436ec3), whose hook at this exit was removed until this is fixed.

`adhoc-group-and-max-typing` (bug, `class-none` until the cause is traced; tag `lang`).
Summary: "the ad hoc path types `Group` and `Max` outside kernel §3". `Group` typed with key
`Key r1` over a `by` of `Key r1.(C1a | C1c)` (record fields are invariant, so no supertype);
`Agg { Max }` typed `Option I64` over a `NonEmpty` source, where §3 gives `ty(body)`. Repros,
with the `norm::adhoc` hook restored: adtb-testkit soundness
`generated_case_results_inhabit_their_types` and `conformance_results_inhabit_their_types`.

Defect class: none until the cause is traced.

## Resolved

Resolved by structure-lang-identity step 8b-1, ptlssrnv 81e1b869147c (4 rounds, full check 0, 3769 tests). Cause: case reduction narrows a `Group`'s `by`, and inlining a function into an aggregate range makes `min`/`max` range over a `Set⁺` while the checked type stays `Option`. Ruled: a rebuilt node keeps the type it was checked at (kernel.md §7); a `Group` may widen its `key` over `ty(by)`; `min`, `max`, `avg` and user aggregates over a `Set⁺` source may be typed `ty(body)` or `Option ty(body)` (kernel.md §3). The `norm::adhoc` hook (`assert_normal` over `NormalAdhoc::roots()`) is restored. Tests: check/tests/adhoc.rs `a_grouping_by_a_key_a_reduction_narrows_keeps_its_checked_type`, `a_match_on_a_key_a_reduction_narrows_keeps_its_checked_type`, the Group refusal cases in kernel_tests.
