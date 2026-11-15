---
created-at: 2026-11-14
status: resolved
closed: 2026-11-14
kind: bug
priority: 370
summary: "translating a registered call to a later version breaks kernel section 3"
tags:
  - lang
  - class-none
---

# translating a registered call to a later version breaks kernel section 3

## Report

Found by the kernel validator of structure-lang-identity step 8a (a1436ec3), whose hook at this exit was removed until this is fixed.

`translate-output-ill-typed` (bug, `class-none` until the cause is traced; tag `lang`).
Summary: "translating a registered call to a later version breaks kernel §3". Seen at the
removed `Stmt::new` hook: `KeyOf(notes, _) : Key notes.Plain`; `Field(_, "who") : Key
identities.(EmailPass | Google)` over a field declared `Key identities`; `Field(_, "tone") :
Tone.Ink`; `Field(_, "n")` over a row type `T.()`, an empty constructor set. Repros:
`adtb-sim` `bundles::a_hiding_pin_handles_known_unique_conflicts_and_refuses_hidden_ones`,
`bundles::each_binder_with_each_access_of_tone_is_stale_after_enable`, with an `assert_valid` at
the exit of `translate/walk.rs`'s entry. Cause to trace; the fix adds that exit hook.

Blocks moving translate/walk.rs onto typed constructors (structure-lang-identity step 8b).

Defect class: none until the cause is traced.

## 2026-11-14

Also blocks the plan-exit hook (`plan/validate.rs`): translated registered calls are planned, and the hook sees the same terms (`KeyOf(notes, _) : Key notes.Plain`, `Field(_, "who") : Key identities.(EmailPass | Google)`, `Field(_, "n")` over `T.()`, `Singleton : Set Note.Plain` over a `Note`, `Field(_, "tone") : Tone.Ink`), via `Stmt::eval` -> `plan::lower::plan` -> `plan::validate::validate`. With the hook in, 7 suite tests and 13 scripts under crates/adtb-sim/scripts/bundles fail. The fix restores both the `Stmt::new` and the plan-exit hooks; the plan-exit hook's lines are in the plan-exit-valid track's receipt.

## Resolved

Landed as ookywzux 9751992c (3 rounds, full check 0, 3778 tests). Ruled: narrowing for a pinned reader happens at the dereference: `Kind::Deref` carries `admits` (within the key's set; a row outside faults `NoSuchRow`; kernel.md §3), keys keep declared types, the engine lookup and permission evaluation read `admits`, and `Totality` treats a narrowed `Deref` as partial. Causes fixed: keys narrowed under `hide new constructors` (`Map::ty_at`); value sums narrowed to the pin (now declared, with `Unreachable` arms); a field over an empty constructor set (now a `Case` with no arms); arm variables typed from the scrutinee (now from their arm's constructor, `arm_var`); composite keys relaid by field identity (`key_layout`). Translate builds every node through `kernel::Build`; `add_unchecked` and the `kernel_door` row are gone; the translate exit, plan exit and `Stmt::new` hooks are live with no skip. Tests: five translate family tests, kernel refusal cases, `a_hiding_pin_dereference_requires_reading_the_pinned_constructors_only`, bundles scripts `hiding-pin-needs-no-read-of-hidden-constructors` and `hiding-pin-admits-a-new-constructor-it-cannot-read`. Filed beside: unreachable-is-total, plan-hiding-unset, parse-accepts-from-type.
