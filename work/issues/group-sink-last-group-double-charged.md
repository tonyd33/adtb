---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 868
summary: "the Group sink keeps the last group's members charged while emitting it, so a grouping fails its grant only when its largest group sorts last"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A grouping whose largest group sorts last exceeds a grant the same data in other order fits

## Report

In the spilled `Group` sink at `crates/adtb/src/exec/sink.rs:360-377`, each group change calls `members_held.release(usize::MAX)` before `emit` (`:368-369`), but the trailing `if let Some((k, ms)) = cur { emit(k, ms)? }` (`:373-375`) emits with `members_held` still charged. The last group's members count twice, once held and once in the emitted record, so whether the call fits its grant depends on key order.

Fix: release before the final emit too, or fold the trailing emit into the loop.

## 2026-10-27: done

The trailing emit releases the last group's members first. Test `whether_a_grouping_fits_its_grant_does_not_depend_on_where_its_largest_group_sorts` fails without the fix.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qexec track; what stays open is in `exec-scan-follow-ups`.
