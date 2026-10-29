---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 850
summary: "a dropped unique, or a relaxing rekey's old key, is refused while a reader holds it instead of applying and staying pending"
branch: pending-unique
tags:
  - engine
---

# Feature: keep a held unique pending

## Report

`evolution.md` section 6.4 lets a drop held by a reader apply and stay pending until its last holder is released. Constraints and checks do this; a `drop unique`, and the old key of a rekey that relaxes it, are refused while held, because the physical index must stay alive until the drop takes effect and be freed at reopen. The ignored test `a_dropped_unique_stays_pending_while_a_reader_pinned_before_the_drop_is_live` in `crates/adtb-sim/tests/write_states.rs` states it. The relaxing rekey case has no test yet.

## 2026-10-28: dispatched

A fresh writer owns this in workspace pending-unique, on main cd020ae0, beside permission-successor (adtb-lang) and the other new track. Local brief: work/briefs/brief-pending-unique.md.

## 2026-10-28: interrupted by a machine crash

The writer had started its full check (target/check-pending-unique.log stops after clippy) when the machine crashed. It left no receipt or handoff. The draft is uncommitted in workspace pending-unique at 30bb1467 (6 files, about 675 lines added), unverified. A fresh writer verifies and finishes it with capped build jobs and no full check; the lead runs the full check after.
