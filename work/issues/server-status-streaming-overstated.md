---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 1012
summary: "server.md and execution.md say a task's single read-only set query is produced a unit at a time, but reads of temp, volatile or file rows and generating queries are formed whole"
closed: 2026-10-29
tags:
  - quality
  - design
---

# Server status streaming overstated

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0), checking docs/design/ against `crates/adtb/src/api/server/call.rs`.

`docs/design/server.md:496-498` says "A task's single read-only set query is produced a unit at a time under supplied capacity", and `docs/design/execution.md:517-519` says "A server task's single read-only query whose root forms a set is produced a unit at a time". `Call::read` (`crates/adtb/src/api/server/call.rs:258-296`) falls back to forming the result whole when `Database::produce` returns `None`, which it does for a query reading temp, volatile or file rows, one that generates values, a `Change` or a `transact` (`crates/adtb/src/write/stream.rs:43-67`), and `exec::produce` forms it whole when the compiled root is not a set-sinking query (`crates/adtb/src/exec/stream.rs:78-81`). A transport sizing its buffers from the design text would expect production bounded by capacity for those queries too; they are bounded only by the grant.

Fix: state the exceptions in both design files (or the rule that decides them), next to the existing note that a pipeline with no heap source is one unit.

## 2026-10-29: resolved

Landed in bb507838..b085c460 (task-fixes) after a clear fresh review (work/briefs/review-task-fixes.md, local) and a lead follow-up guarding on `drained && produced`. Branch check on 30bc4d3b passed (root 2782 passed, 7 skipped; campaign 47); main check skipped, the landed parent differing from the base only in tickets.

## 2026-10-29: resolved
