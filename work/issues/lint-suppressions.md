---
created-at: 2026-10-28
status: resolved
kind: feature
priority: 120
summary: "about 87 lint expects and 7 size-allowlist entries stand in the source, and each one is a precedent other agents copy"
closed: 2026-10-29
tags:
  - quality
---

# Feature: minimize lint suppressions

## Report

Main 28bacb53 has about 87 `#[expect(..., reason = ...)]` attributes: dead_code about 38, clippy::too_many_lines 24, clippy::disallowed_methods 10, clippy::print_stderr and print_stdout 8, clippy::cognitive_complexity 5, and a few others; adtb-sim holds about 33 and adtb-lang about 13. work/tools/size-allowlist exempts 7 files from the source-size limit. Agents read these as license to suppress. Keep only suppressions where the rule is wrong for that spot, each with a reason stating what would remove it, and stop new ones spreading.

## 2026-10-28: audit dispatched

A read-only auditor (workspace lint-audit, no builds) classifies every suppression as remove, narrow or keep, groups fixes into batches by the files running tracks own, and recommends one prevention mechanism. Report: work/briefs/report-lint-audit.md (local); the lead copies the verdicts and batches here before dispatching fix tracks. Local brief: work/briefs/brief-lint-audit.md.

## 2026-10-28: audit result

87 attributes: remove 70, narrow 3, keep 14; all 7 size-allowlist files can split at a real seam. 36 of the 38 dead_code expects come from support modules pulled into each sim test binary by `#[path]`/`mod`; the fix is one `publish = false` support library crate (as sim-api-support-dead-code prescribes, folding in sim-test-helper-duplication), with unused moved helpers deleted. The planner fixture shared by path gets a `plan-fixture` feature in adtb with adtb-reference and adtb-testkit as optional deps (lead ruling: accepted). Keeps: flat single-match dispatchers (exec/code.rs run and expr, exec/prim apply, reference eval and apply), the sim write case table, host threads in the simulator and one adtb-sys wrapper, CLI prints in adtb-probe and adtb-conformance, ignored host-time benchmarks, and codegen's emitted non_camel_case_types. About 50 reasons state only why the lint fires; rewrite each keep's reason to say what would remove it. faults.rs:142 expects a lint that is off (pedantic cast_precision_loss). code-agent.md's line on `#[expect]` reads as permission.

Prevention: house-check flags added `#[expect(` lines and size-allowlist entries; code-agent.md says to fix the code and add an expect only when the rule is wrong for that spot, naming it in the report.

Batches: 1 (free now) sim support crate and plan-fixture, sim prints and the query table, engine choose/prune/boundary/lower args and the plan/lower split, adtb-sys host_thread and faults and redundant denies, pages.rs narrowing, keep reasons, prevention. 2 (after reference-recursion) soundness.rs:480 and recheck of the reference keeps. 3 (after version-translation) the 18 adtb-lang rows, six adtb-lang splits and soundness.rs:4. Full table: work/briefs/report-lint-audit.md (local).

## 2026-10-28: batch 1 dispatched

A fresh writer owns batch 1 in workspace lint-audit, rebased on main, outside the files the running tracks own. Gates: full suite, clippy, fmt, house-check, size-check and the expect count before and after; the lead runs the full check. Local brief: work/briefs/brief-lint-fix.md.

## 2026-10-29: batch 1 landed

Landed as 6686d1cb after a clear review (work/briefs/review-lint-fix.md, local); the main check passed on it. Expects went from 87 to 37: sim test support moved into crates/adtb-simkit, the planner fixture is behind adtb's `plan-fixture` feature, the engine functions the audit named were split, adtb-sys spawns through `host_thread`, and house-check flags a new expect. Batches 2 and 3 of the audit remain. Added to batch 2:
- crates/adtb-reference/tests/wide.rs:13, landed after the audit; its reason states only the cause.
- work/tools/house-check-test has no cases for the two new rules.
- docs/design/README.md: the `adtb` row does not name the optional `plan-fixture` dependencies.
- .agents/skills/lead/code-agent.md: restore "never add `#[allow]`" in the reworded expect line.
- Seven adtb-sim test binaries each define a near-identical `query(db, text)`; fold it into adtb-simkit.

## 2026-10-29: batch 2 landed

Landed as cb259572; the main check passed on it. Expects went from 37 to 36: the soundness fuzz test runs on the harness thread, the kept reasons in the reference say what removes them, house-check-test covers the new rules, the design README names the `plan-fixture` dependencies, code-agent.md again says never to add `#[allow]`, and `query` is folded into adtb-simkit except in bundles.rs. Batch 3 (the adtb-lang rows and splits) waits for version-translation; it also folds the bundles.rs `query`.

## 2026-10-29: batch 3 dispatched

version-translation landed, so batch 3 runs in workspace lint-batch3 on main 0c489c2c: the audit's adtb-lang rows and size splits except under crates/adtb-lang/src/check/ (checker-match-recursion edits it), moving crates/adtb-lang/tests/running into adtb-testkit, folding the bundles.rs `query`, and any expect version-translation added. The check/ rows follow in a later batch. Local brief: work/briefs/brief-lint-batch3.md.

## 2026-10-29: batch 3 landed

Landed as 482661ac after a clear review (work/briefs/review-lint-batch3.md, local); the main check passed on it. Expects went from 36 to 21: the adtb-lang rows outside check/ are done, evolve/ctor, evolve/mod.rs, evolve/revert and schema.rs are split into submodules, 13 enums box their large variants (one expect kept on `WriteKind`), and crates/adtb-lang/tests/running and its corpus moved into adtb-testkit as `running` and `spec`.

Batch 4:
- the check/ rows: check/prelude.rs `prelude_call`, check/expr.rs `build_ctor`, check/mod.rs `bodies`, and the check/expr.rs and check/write.rs size splits;
- narrow 34 `pub(super)` items that compile private (18 in evolve/cx.rs, 3 in evolve/source.rs, 13 in schema/build.rs) and evolve/revert/inverse.rs `inverse` to `pub(super)`;
- schema/relation.rs about line 321: the doc omits `None` for an unknown constructor;
- doc wording at evolve/ctor/merge.rs:116, split.rs:135 and split.rs:12;
- evolve/ids.rs free `relations_of` (paths) beside `Cx::relations_of` (tuples): rename one.

## 2026-10-29: batch 4 dispatched

Workspace lint-batch4 on main df51cb14 takes the batch 4 list above. Local brief: work/briefs/brief-lint-batch4.md.

## 2026-10-29: batch 4 landed

Landed as 187dcb6c after a clear review (work/briefs/review-lint-batch4.md, local); the main check passed on it. Expects went from 21 to 18 and work/tools/size-allowlist is empty: check/expr.rs is split into record.rs, call.rs and query.rs, check/write.rs into txn.rs, `prelude_call` dispatches over eight family checkers, `bodies` is a `Bodies` struct, and 38 items are narrowed to private. Kernel output was byte-identical over all conformance schemas and texts. The audit's rows are all done; 15 moved doc comments are still in third person.

## 2026-10-29: resolved
