---
created-at: 2026-10-29
status: open
kind: bug
priority: 565
summary: "checked terms still bind one variable at several sites outside patterns, and the normaliser's For fusion and copy of the copyable test can drift"
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
