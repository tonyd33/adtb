---
created-at: 2026-11-03
status: open
kind: bug
priority: 560
summary: "the revert search has no pair for inverse steps that apply no operator, a key clause, or a multi-operator revert with a crossed operator; Pinned::new replays every revert per apply"
---

# Search the reverts translation judges by id

## Report

From translate-revert. A revert's inverse steps that apply no operator (Unadd, Readd, States, Decl, Module, KeyClause, Reorder, Grants) are judged by catalog id through `Map`, and blame falls back to the revert's text. The search in crates/adtb-lang/tests/pinned-revert/ has no pair for the revert of remove constructor (Readd, States), enable or deprecate (States), alter field, drop type or relation (Decl), module operators, grants, a rekey exercising KeyClause, or a revert spanning several operators of which one is crossed. Add those pairs with the written inverse as oracle. Also: `Pinned::new` (translate/mod.rs `reverted`) rebuilds two schemas per inverse step for every revert since the pin, per live bundle, per apply and registration; cache per migration if it shows in a profile.
