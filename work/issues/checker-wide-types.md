---
created-at: 2026-10-29
status: open
kind: bug
priority: 680
summary: "a function of thousands of parameters overflows Ty::subst, and a match on a sum of thousands of constructors checks in quadratic time"
---

# Bug: very wide function types and sums

depends on: none
needed by: none

## Report

Found by the checker-match-recursion writer (work/briefs/receipt-checker-recursion.md, local):

1. Function types nest one level per parameter, so `Ty::subst` and the derived `Clone`, `Drop` and `PartialEq` recurse once per parameter. A function of 4000 parameters overflows in `Ty::subst`; the parser does not limit the parameter count.
2. A match on a sum of 4000 constructors takes about 20 s to check at opt-level 0: the coverage work is quadratic in constructor count.

## Fix

Either bound the parameter count in the parser, as `MAX_DEPTH` bounds nesting, or encode function parameters as a list in crates/adtb-lang/src/types.rs. Make coverage over a sum's constructors linear by indexing rows by constructor.
