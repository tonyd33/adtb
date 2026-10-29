---
created-at: 2026-10-29
status: open
kind: bug
priority: 780
summary: "semantics.md 10.2 still gives the old match desugaring chain, and three internal errors still say only exhaustiveness excludes an Unreachable"
---

# Bug: match retest follow-ups

depends on: none
needed by: none

## Report

From the review of the match retest lowering (work/briefs/review-match-retest.md, local):

- docs/spec/semantics.md about lines 1537-1567 (section 10.2) gives the old `P(y, p, K, F)` chain as the desugaring. The new lowering means the same; decide through the spec-revision workflow whether the spec should show it or state only the meaning.
- Internal-error texts still say "exhaustiveness excludes" where docs/design/kernel.md now also allows a dominating test: crates/adtb-reference/src/eval.rs about line 817, crates/adtb-reference/src/stmt.rs about line 379, crates/adtb/src/write/eval.rs about line 538.
- `a_nested_pattern_cases_on_the_field` and `a_guarded_chain_ends_unreachable` (crates/adtb-lang/src/check/tests/mod.rs) do not assert the test's arms or the bodies' contents in full.
- Guard variables gain a binding site per or-pattern alternative; decide with or-pattern-binding-sites.

## Fix

Reword the three error texts, assert the two tests' terms in full, and take the spec wording through spec-revision.

## 2026-10-29: code items landed

The three error texts, the two tests asserted in full (`a_guarded_chain_ends_unreachable` renamed `a_false_guard_falls_through_to_the_next_arm`) and the guard variable binding sites landed with or-pattern-binding-sites in fe0855c7. Only the semantics.md section 10.2 wording remains, for spec-revision.
