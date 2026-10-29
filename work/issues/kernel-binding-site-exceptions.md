---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 565
summary: "checked terms still bind one variable at several sites outside patterns, and the normaliser's For fusion and copy of the copyable test can drift"
branch: binding-sites
---

# Bug: kernel variables with several binding sites outside patterns

depends on: none
needed by: none

## Report

From the or-pattern-binding-sites track (landed fe0855c7; receipts work/briefs/receipt-or-patterns.md and reviews review-or-patterns*.md, local). docs/design/kernel.md section 2 states one binding site per variable; the corpus check asserts it only on normal forms per root because:

- `Present`, `Top` and `Agg` reuse the variable their `For` source binds in checked terms.
- The group desugaring binds one variable at a `Let` and a `For`, or at two `Let`s; an aggregate test binds one at a `Lam` and a `For`.
- The normaliser's `for_over` (crates/adtb-lang/src/norm.rs about 1253) reuses the inner `For`'s variable, so two roots sharing an arena bind it twice, and likely one root too (`count(fs()) + count(from x in fs() select key x)`).

Also: `Facts::copyable` and `draws` (crates/adtb/src/plan/terms.rs) copy the normaliser's private pair and can drift; no test reaches the copyable guard with a `Fresh` or non-deterministic bound; `sole_ctor` does not handle `Ty::Tag`.

## Fix

Give each site its own variable, assert the rule on checked terms, and share one copyable predicate between norm and plan.

## 2026-10-29: dispatched

A fresh writer in workspace binding-sites on main 2125596c959f gives every site its own variable, runs the one-binding-site assertion on checked terms and normal forms over the checker corpus, and shares one copyable predicate between norm and plan. Results unchanged across conformance, differential and plan tests.

## 2026-10-29: review sends it back

Frozen 5b795cf5 gives every site its own variable, asserts the rule on checked terms and normal forms across the corpus, and shares `Totality::copyable`/`draws` between norm and plan. A fresh review (work/briefs/review-binding-sites.md, local) found a size blowup: `Terms::renamed` copies every subterm reading a variable bound under its root, so each use of a group binder copies the whole group value; nested groups double distinct nodes per level where base was linear. Lead ruling: copy a subterm only when it has a free variable in the current renaming, with memoised free-variable sets, in `renamed` and in the normaliser's `bound_afresh`; a distinct-node test at two depths.

## 2026-10-29: delta review sends it back again

ca6c226c (rebased 86967099, branch check passed: root 2723 passed, campaign 47) shares closed subterms in `Terms::renamed`, making the closed group witness linear. A second review found a group value that reads an outer generator is still copied whole at each use, so nested dependent groups double checked nodes per level where base shared one value per use. Lead ruling: bind a group value once as a function of the row and apply it at each use; the same for the other per-use copies; distinct-node tests at three depths with a constant increment per level, including a variant whose normal form base kept linear.
