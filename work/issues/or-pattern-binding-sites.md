---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 560
summary: "an or-pattern binds one kernel variable at several exclusive sites, against the one-binding-site rule of design kernel.md section 2"
closed: 2026-10-29
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

## 2026-10-29: review sends it back

Frozen 8718388c passed the lead's branch check (root 2641 passed, 7 skipped; campaign 47) and a fresh review found no wrong answer, but the selector `If` is copyable and inlined at every use: with u uses, m alternatives in one constructor binding different paths and tests of size T, tree and evaluation cost are about u·m·T against base m·(u+T). Lead ruling: a selector is evaluated once per arm entry, so it stays let-bound in the normal form. Also fixed in the same pass: `sole_ctor` misses single-constructor types written `ctors: None` or `sel: None`; a nested constructor set still binds per-alternative variables at several sites in checked terms; `one_binding_site` skips programs norm refuses; test names and coverage. The normaliser's `for_over` reuse of the inner `For` variable is filed separately.

## 2026-10-29: delta review blocks 53745f9b

53745f9b keeps every copyable `Let` whose bound is an `If` or `Case`, not only selectors. The lead's branch check passed (root 2645 passed; campaign 47), but a fresh delta review found a lost index probe and a constraint base admitted now refused, when a function is called with a branching argument: plan/lower `peel`/`conjuncts` and the incremental-check analysis do not see through the kept `Let`. Lead ruling: substitute a branching copyable bound used at most once, keep it only when used more than once, and make `peel`/`conjuncts` and the delta analysis see through a kept copyable `Let` at a condition's root; the reviewer's two witnesses and a two-use variant become crates/adtb tests.

## 2026-10-29: third review blocks 5263f185

5263f185 substitutes a branching bound used once and sinks a kept copyable `Let` onto both sides of an equality conjunct (`Facts::sink_let`, crates/adtb/src/plan/terms.rs). A fresh review found a range probe lost: `fn near (a: I64, b: I64) -> Bool = a >= b and a < b + 10` called with a branching argument plans a scan where base planned `Range probe ms (n)`. Lead ruling: `sink_let` distributes over every scalar operator, not only `Eq`, and is guarded on copyable rather than total; the `near` witness becomes a plan test.

## 2026-10-29: landed

Landed as fe0855c7 after four reviews; the fourth cleared c10f488b. Branch check on 1f9f1cc5 (parent a4e1b45f, the landed parent) passed: root 2648 passed, 7 skipped; apps/campaign 47 passed, 1 skipped; all static checks passed. Each or-pattern name is bound once by a `Let` over a selector `Case` that returns the matched alternative's value; the normaliser keeps a branching copyable bound as a `Let` only when it is used more than once; `Facts::sink_let` distributes a kept copyable `Let` over every primitive at a condition's root in plan/lower and write/delta, so probes and admissions match base. Remaining binding-site exceptions are in kernel-binding-site-exceptions.

## 2026-10-29: resolved
