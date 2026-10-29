---
created-at: 2026-10-28
status: in-progress
branch: lint-audit
kind: feature
priority: 120
summary: "about 87 lint expects and 7 size-allowlist entries stand in the source, and each one is a precedent other agents copy"
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
