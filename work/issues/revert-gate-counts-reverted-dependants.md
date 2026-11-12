---
created-at: 2026-11-12
status: open
kind: bug
priority: 250
summary: "the revert dependency gate counts a dependant that a later revert already undid, and the spec does not say whether a reverted migration is applied"
tags:
  - class-none
---

# A reverted dependant still blocks a revert

## Report

0001 `add constructor T.C`, 0002 `enable ts.C`, 0003 `revert 0002`, then `revert 0001` is refused naming 0002, though 0002's effect is undone. evolution.md §5.4 refuses "while a dependant of n is applied"; whether a migration a later `revert` undid (and no later revert re-applied) is still applied is undecided. Same on main; found in the structure-lang-identity step 4 review.

Direction: a spec ruling first (a reverted migration is not applied, unless a later revert of its revert re-applies it), then the gate follows the chain. Test: the scenario applies after the ruling, and a chain of revert of revert still blocks.

Defect class: none.
