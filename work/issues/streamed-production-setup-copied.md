---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 1010
summary: "Database::produce and exec::produce copy the statement path's planning, error mapping, compile, host binding and context setup"
closed: 2026-10-29
tags:
  - quality
  - engine
---

# Streamed production setup copied

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0).

- `crates/adtb/src/write/stream.rs:69-83` (`Database::produce`) repeats `Eval::plan_in` (`crates/adtb/src/write/eval.rs:378-391`): the same `crate::plan::plan` call with default options, the same `plan::Error::Internal` to `Error::Internal` / otherwise `Error::Unsupported(e.to_string())` mapping, then `exec::compile` and `program.shared.hosts = self.hosts.clone()` as `eval.rs:363-365` does. The host binding is a ninth site of the pattern host-binding-after-construction lists, so a fix there must cover it.
- `crates/adtb/src/exec/stream.rs:66-85` (`exec::produce`) repeats `exec::execute`'s `Ctx` construction (`crates/adtb/src/exec/mod.rs:275-286`, differing only in `pool: None`) and `Ctx::root`'s frame binding of the parameters (`crates/adtb/src/exec/run.rs:113-119`).

Fix: give the plan-and-compile step one function on `Database` used by `Eval` and `produce`, and build the `Ctx` and the bound frame in one helper `execute` and `produce` share, the pool passed in.

## 2026-10-29: resolved

Landed in bb507838..b085c460 (task-fixes) after a clear fresh review (work/briefs/review-task-fixes.md, local) and a lead follow-up guarding on `drained && produced`. Branch check on 30bc4d3b passed (root 2782 passed, 7 skipped; campaign 47); main check skipped, the landed parent differing from the base only in tickets.

## 2026-10-29: resolved
