---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 1010
summary: "Database::produce and exec::produce copy the statement path's planning, error mapping, compile, host binding and context setup"
branch: task-fixes
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
