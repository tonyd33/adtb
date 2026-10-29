---
created-at: 2026-10-29
status: open
kind: bug
priority: 425
summary: "the planner, code generation, constraint deltas and the permission evaluator do work once per path through a shared kernel subterm, exponential in a chain of inlined functions"
---

# Kernel path sharing

## Report

depends on: none
needed by: none

## Report

From the kernel-dag-tree-walks track (landed aa95dce5; receipt work/briefs/receipt-dag-walks.md, local). Inlined function bodies share subterms, and these passes carry path state, so a visited set does not apply. Witnesses are chains such as `fn d{i+1}(x) = d{i}(x) + d{i}(x)` and `fn u{i+1}(x) = if x > i then u{i}(x) else u{i}(x)`.

- plan/lower `spine`, `chain`: one plan subtree per path. `from r in raws, y in u{n}(r.n) select y` explains 16,386 / 65,538 / 262,146 lines at n = 12 / 14 / 16.
- plan/lower `eval`: through non-scalar terms. `from r in raws select count(y for y in u{n}(r.n))` gives 1,031 / 16,391 lines at n = 8 / 12.
- plan/lower `delta`: a memo would fit, but `eval` and `spine` expand its result per path again.
- exec/code.rs `expr`, `block`: instructions per path. `from r in raws select d{n}(r.n)` takes 0.58 / 0.80 / 1.41 s at n = 10 / 16 / 18. Needs common-subexpression reuse scoped by block, careful with `Try` and `Bind`.
- write/delta `top`, `nested`: an occurrence per path. `constraint c = from r in raws, k in h{n}(r.n) select r` opens in 0.53 / 0.60 / 1.07 / 7.46 s at n = 4 / 6 / 8 / 10 before it is refused.
- write/delta `unfold`: a `HashMap<Tm, Tm>` memo would fit; no admitted witness found.
- permission/eval.rs `eval`: `from r in left select d{n}(r.id)` takes 7 / 22 / 86 / 293 ms at n = 12 / 14 / 16 / 18.

Also from the review: aggregate `selects` is still exponential when a shared term sits under different chains of generators; `write_depends_on_volatile` is quadratic, since `dirty` and `writes` start a fresh walk at each binder and branch; no new test has a hit under a shared term.

## Fix

Decide per pass between a memo keyed by term and path state, sharing in the output (plans and code as DAGs), or bounding inlining. Add a test of each witness.
