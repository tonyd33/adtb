---
created-at: 2026-11-08
status: resolved
kind: bug
priority: 720
summary: "Evidence::unrefuse and restore lift or reset a page's refusal from a bare Pid, while refusing needs a Refusal made from the page's latch"
closed: 2026-11-08
tags:
  - quality
  - engine
---

# A page refusal can be lifted without the page latch

## Report

Refusing a page needs a `Refusal`, built only from `&Held` (crates/adtb/src/storage/evidence.rs:111), so a refusal is set only under the page's latch. Lifting or resetting one does not: `Evidence::unrefuse(pid)` (evidence.rs:541) and `restore(pid, damage)` (evidence.rs:550) take a bare `Pid`. Every caller complies today (crates/adtb/src/storage/pager.rs:682, 703, 713, 721 under a held latch; crates/adtb/src/storage/recovery.rs:266 during single-threaded replay), by convention only. A later caller lifting a refusal without the latch lets a reader past `Pager::refused` onto a frame being rewritten.

Fix direction: `unrefuse` and `restore` take the `&Held` (or the `Refusal`) of the page, and recovery's chain start the `Replaying` witness.

## 2026-11-08: resolved

`Evidence::unrefuse` and `restore` take the page's `Refusal<'a>`, which borrows the held latch, so the latch is held where the refusal changes, and replay's chain start lifts a refusal through `replay_start`, which takes the `Replaying` witness.
