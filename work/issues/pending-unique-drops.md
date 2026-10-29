---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 850
summary: "a dropped unique, or a relaxing rekey's old key, is refused while a reader holds it instead of applying and staying pending"
closed: 2026-10-28
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

## 2026-10-28: resumed writer froze the source

Frozen 79554f8da1df2fe5b1b121a208c7e0f7e529773f on main cd020ae0 (workspace pending-unique). The crashed draft was complete for `drop unique`; the resumed writer added the relaxing rekey's old key, which stays as a physical unique index while held, and removed the old refusal. It also made write/schema.rs `rels` name an index the schema does not declare by its key clause.
Targeted gates passed: suite over write_states, bundles and package(adtb) 449/449 with ignored tests run; strict workspace clippy, fmt, house-check, size-check. No test is left ignored. Receipt work/briefs/receipt-pending-unique.md (local).
Running: the lead's full branch check and a fresh independent review.

## 2026-10-28: lead full branch check

State: /home/tony/code/adtb-pending-unique, tested 79554f8da1df2fe5b1b121a208c7e0f7e529773f, base cd020ae0; commit id unchanged after the run.
Outcome: passed, exit 0. Root 2454 passed, 7 skipped (one fewer skipped than main: the un-ignored pending unique test); apps/campaign 47 passed, 1 skipped; clippy, fmt, deny, size-check and docs-check passed; 3m12s. Log target/check-lead.log in the workspace.
Remaining: the independent review (running), landing, main check.

## 2026-10-28: landed

Independent review (work/briefs/review-pending-unique.md, local) found nothing blocking; its non-blocking findings are in pending-unique-follow-ups. Landed as 7c524015 on main. Main check passed, exit 0: root 2518 passed, 7 skipped; apps/campaign 47 passed, 1 skipped; 6m42s. Log target/check-main-pending-unique.log.

## 2026-10-28: resolved
