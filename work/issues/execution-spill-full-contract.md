---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 895
summary: "execution.md says a spill I/O failure is never StatementLimit, but a full device is reported as StatementLimit Memory"
closed: 2026-10-27
tags:
  - quality
  - design
---

# execution.md says a spill I/O failure is never StatementLimit, but a full device is

## Report

`docs/design/execution.md:701-702` says a spill file that cannot be made, written or read fails the statement with that I/O error, not `StatementLimit`. `Spill::failed` (`crates/adtb/src/exec/spill.rs:65-74`) maps `IoError::Full` to `StatementLimit(Exceeded::Memory)`, following the ruling recorded in `exec-scan-follow-ups`. The design file is the stale side.

Fix: state the `Full` exception in `execution.md`.

## 2026-10-27: done

`docs/design/execution.md` now states that `IoError::Full` on a spill file fails the statement with `StatementLimit(Exceeded::Memory)` at the grant limit.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qwrong track.
