---
created-at: 2026-10-27
status: open
kind: bug
priority: 868
summary: "the Group sink keeps the last group's members charged while emitting it, so a grouping fails its grant only when its largest group sorts last"
tags: [quality, engine]
---

# A grouping whose largest group sorts last exceeds a grant the same data in other order fits

## Report

In the spilled `Group` sink at `crates/adtb/src/exec/sink.rs:360-377`, each group change calls `members_held.release(usize::MAX)` before `emit` (`:368-369`), but the trailing `if let Some((k, ms)) = cur { emit(k, ms)? }` (`:373-375`) emits with `members_held` still charged. The last group's members count twice, once held and once in the emitted record, so whether the call fits its grant depends on key order.

Fix: release before the final emit too, or fold the trailing emit into the loop.
