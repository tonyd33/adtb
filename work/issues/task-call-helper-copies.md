---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 1011
summary: "the task call copies arrow.rs internal and server denied, and flattens a typed grant error to Debug text for an infallible take(0)"
closed: 2026-10-29
tags:
  - quality
  - engine
---

# Task call helper copies

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0).

- `crates/adtb/src/api/server/call.rs:727-731` `internal` is a copy of `crates/adtb/src/api/arrow.rs:187-191` `internal`.
- `crates/adtb/src/api/server/task.rs:355-358` builds `Error::rejected(RejectReason::Denied { .. })` inline where its parent module has `denied` (`crates/adtb/src/api/server/mod.rs:260`).
- `Call::arena` (`call.rs:342-345`) maps the `exec::Error` of `grant.take(0)` through `internal(&format!("{err:?}"))`. `take(0)` cannot fail (`Grant::reserve` admits zero bytes while `used <= limit`), and were it to fail with `StatementLimit` it would reach the client as `Internal` Debug text where every sibling grow maps through `e.write_error`.

Fix: use the existing helpers; give `Grant` an infallible empty-arena constructor (or map through `write_error` as the `grow` calls do) and drop `internal` from call.rs.

## 2026-10-29: resolved

Landed in bb507838..b085c460 (task-fixes) after a clear fresh review (work/briefs/review-task-fixes.md, local) and a lead follow-up guarding on `drained && produced`. Branch check on 30bc4d3b passed (root 2782 passed, 7 skipped; campaign 47); main check skipped, the landed parent differing from the base only in tickets.

## 2026-10-29: resolved
