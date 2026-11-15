---
created-at: 2026-11-14
status: open
kind: bug
priority: 150
summary: "the planner's hiding option is never set"
tags:
  - plan
  - class-none
---

# the planner's hiding option is never set

## Report

Found by the translate track (ookywzux 9751992c), confirmed by its review. `plan::Options::hiding` is never set true (`rg 'hiding: true'` finds nothing outside tests; write/eval.rs and catalog/validate.rs pass `plan::Options::default()`), so translated calls of a bundle hiding new constructors are planned with it false. The review saw no client-visible divergence on main (`exists` and `count` over a session referencing a hidden row fault `NoSuchRow` either way), and since 9751992c `Totality` makes a dereference with a narrowed `admits` partial, so the option no longer decides translated dereferences. Remove the option and the code it guards, or set it where it is meant to hold; state which with a test.

Defect class: none (a dead option).
