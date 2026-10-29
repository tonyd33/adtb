---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 420
summary: "Terms::any, writes, reads_only_row and several analysis walks treat the kernel DAG as a tree, so any shared subterm is visited once per path"
branch: dag-walks
---

# Bug: kernel passes walk the term DAG as a tree

depends on: none
needed by: none

## Report

Found while analysing match-failure-duplication (work/research/notes/match-failure-duplication-evidence.md). Kernel terms are a DAG (docs/design/kernel.md section 2), but these walks have no visited set or memo, so a shared subterm is visited once per path: `Terms::any` and `writes` (crates/adtb-lang/src/kernel.rs about lines 346 and 359, called on every normal form from crates/adtb/src/api/text.rs:119), `reads_only_row` (check/mod.rs about line 825), `write_depends_on_volatile` (analysis/volatile.rs:23), analysis/aggregate.rs, translate/walk.rs `yields`, permission/eval.rs. Path-sensitive passes (plan/lower `eval`, `spine`, `delta`; exec/code.rs `expr`, `block`; write/explain.rs; write/delta `top`, `nested`, `unfold`) produce output per path, not only time.

## Fix

Give the context-free walks a visited set keyed by `Tm`, starting with `Terms::any` and `writes`. For the path-sensitive passes, list which can meet sharing once match-failure-duplication's lowering lands (inlined bodies remain), with a test of a shared subterm under many paths.

## 2026-10-29: dispatched

match-failure-duplication landed (244b9dc8), so inlined bodies are the remaining source of sharing. A fresh writer in workspace dag-walks on main a917867e gives the context-free walks a visited set keyed by `Tm`, with a test per walk of a shared subterm under many paths that is slow or wrong on base, and lists for each path-sensitive pass whether it can meet sharing, with a witness where it can. Fixing path-sensitive passes is a later track unless the fix is a memo of the same shape. Gates: adtb-lang and the affected adtb tests, clippy, fmt, house-check, size-check; the lead runs the full check.
