---
created-at: 2026-11-13
status: open
kind: bug
priority: 280
summary: "reverting an add constructor that widened a record leaves a sum when a later migration renamed the original constructor"
tags:
  - class-none
---

# Reverting a widening after a constructor rename leaves a sum

## Report

`type V = { id: Key vs, w: Text }`; 0001 `add constructor V.V2 { w2: Text }`; 0002 `rename constructor V.V to V9`; `revert 0001`: the revert removes V2 and leaves `type V = V9 { .. }`, a one-constructor sum, not the record before 0001 (evolution.md §5.1). Restoring the record form would undo 0002's rename, which is not a dependant of 0001 by the gate. Found in structure-lang-identity step 5c-3; no law case reaches it.

Direction: needs a ruling: refuse with a §5.4 reason (the record form names the constructor a later migration renamed), or treat the rename as a dependant. Test: the history above.

Defect class: none yet.

## Later objects naming the constructor

Reverting the widening also leaves later objects that name the record's constructor as written: `default V.w`, `grant update vs.V`, `set SV of vs = V` stay, and the law search reports the default as a new id by description. Whether they are dependants of the widening (they name constructor `V`, which predates it) belongs to the same ruling. Found in the delta review of structure-lang-identity step 5c-3.
