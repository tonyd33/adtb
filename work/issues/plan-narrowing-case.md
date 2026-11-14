---
created-at: 2026-11-14
status: resolved
closed: 2026-11-14
kind: feature
priority: 380
summary: "plan lowering narrows a variable with a Let the kernel cannot type"
tags:
  - lang
  - plan
  - structure
---

# Feature: narrow a variable in plan lowering with a one-arm Case

## Report

Found by the kernel validator of structure-lang-identity step 8a (a1436ec3), whose hook at this exit was removed until this is fixed.

`plan-narrowing-case` (feature, tag `lang`, `plan`). Summary: "plan lowering narrows a
variable with a `Let` the kernel cannot type". Kernel.md §3 binds lowered plan terms;
`specialise` (`plan/lower/mod.rs:544-566`) emits a `Case` with the one live arm and `Unreachable`
for the others in place of `Let(x : T.c, v : T, arm)`. The track compares the plans before and
after (`plan` scripts, `explain` output) and restores the plan-exit hook. Repros, with the hook
restored: `exec::exec_tests::every_program_records_the_type_of_its_root`,
`plan::tests::a_bounded_plan_narrows_a_generator_matched_to_one_constructor` (a variable of
`Identity.Google` bound to, or read as, a term of `Identity`).

Blocks structure-lang-identity step 8b-2.

## Resolved

Landed as xsrlnowt d71a539e with kernel-fresh-placeholder-kind. `specialise` keeps the `Case` with the live arm specialised and every other arm `Unreachable`; `split_join`'s `Restrict` and rename are typed at the variable's type (a third cause the repro hit). A `Case` with one arm that is not `Unreachable` compiles to `Ins::Narrow`: a per-row tag test failing with `Broken::Unreachable` on another constructor, then the bind (exec::tests `a_case_with_one_reachable_arm_takes_it_and_fails_on_another_constructor`). kernel.md §3 states that its rows bind terms a consumer builds, lowered plans included. No plan changed (explain over 11 queries and the plan scripts). The plan-exit hook waits on translate-output-ill-typed.
