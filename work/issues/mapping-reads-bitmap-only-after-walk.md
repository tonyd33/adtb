---
created-at: 2026-11-10
status: open
kind: bug
priority: 420
summary: "the damage mapper reads the allocation bitmap only after the walks, so a page free before them and allocated during them maps as unreached and marks the database once running"
tags:
  - quality
  - engine
  - class-not-atomic
---

# Bug: the damage mapper reads the bitmap only after the walks

## Report

storage.md §8 maps "a page the allocation bitmap calls free, read before and after the walk" to nothing, and the doc of `Pager::holders_of` (crates/adtb/src/storage/quarantine.rs:754) promises "A page the allocation bitmap calls free, before or after the walk, is held as free unless a walk found it". `Pager::holders_walked` (quarantine.rs:794-818) reads the bitmap once, after `walk` returns (`if self.bitmapped(pid) == Bitmapped::Free` at :814). No caller reads it before: `Structures::holders` and `Unopened::holders` (crates/adtb/src/catalog/structures.rs:148, :229) go straight to `holders_meeting` / `holders_walked`, and `bitmapped` has no other mapping caller.

The mapping at the end of a call runs without the serialization lock, beside writers. A damaged page the bitmap calls free when the mapping begins, allocated by a concurrent writer to a structure the walks have already passed, reads as allocated after the walks and is held `Unreached`, which `policy::mark` maps to `Mark::Database` in `Phase::Running`: the database is marked durably for a page that was free when its mapping began. The pre-range code had the same single read; step 3 of structure-damage-policy rewrote the function and kept the doc's "before or after".

Repro direction: a sim test that records damage on a free page, then interleaves an allocation of that page into a heap the walk has finished with the end-of-call mapping, and asserts no Database mark.

Fix direction: read `bitmapped(pid)` before `walk` as well and hold the page `Free` when either read says free, as the doc states; or, if one read is the rule, change storage.md §8 and the method doc through the design space.

Found in the quality review of `reviewed..main` at 7bf40103.
