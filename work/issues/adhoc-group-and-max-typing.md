---
created-at: 2026-11-14
status: open
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
