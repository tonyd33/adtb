---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 560
summary: "an or-pattern binds one kernel variable at several exclusive sites, against the one-binding-site rule of design kernel.md section 2"
branch: or-patterns
---

# Bug: or-pattern variables bound at several sites

depends on: none
needed by: none

## Report

Found while analysing match-failure-duplication. An or-pattern's alternatives bind the same kernel `Var` at several exclusive sites (crates/adtb-lang/src/check/pat.rs about line 1116, under the loop at 1123-1128). docs/design/kernel.md section 2 states one binding site per variable. Passes that key facts by variable may merge the alternatives' facts.

## Fix

Give each alternative its own variables and join them in the body, or state in kernel.md the exclusive-sites exception and the passes that rely on it. Decide together with match-failure-duplication's guard-variable rule (proposed point match-failure-retest).

## 2026-10-29: from the kernel-dag-tree-walks review

`write_depends_on_volatile` (crates/adtb-lang/src/analysis/volatile.rs) walks with a visited set and is correct only because each variable has one binding site in a normal form. If exclusive binding sites reach normal forms, the shared continuation `k` in `Let(x, clean, k)` and `Let(x, volatile_read, k)` is visited once and the dependence is missed. Keep one binding site per variable, or make that walk path-sensitive.

## 2026-10-29: dispatched with match-retest-follow-ups

Lead ruling: kernel.md section 2 keeps one binding site per variable. Each or-pattern alternative binds its own fresh variables, and each pattern variable is bound once by a `Let` over a `Case` on the scrutinee that returns that alternative's variable, with `Unreachable` where the test excludes; the body is not duplicated, so the term stays linear in alternatives. Guard variables follow the same rule. A fresh writer in workspace or-patterns on main d8d9c197 implements it with a one-binding-site check over the checker corpus that fails on base, and also takes the error texts and the two tests of match-retest-follow-ups (not the semantics.md wording).
