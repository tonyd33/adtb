---
created-at: 2026-11-14
status: open
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
