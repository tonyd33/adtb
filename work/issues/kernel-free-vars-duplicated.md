---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 899
summary: "kernel Terms::frees and plan Facts::free each memoise free variables per term, and the kernel one drops a binder from its own bound value"
branch: kernel-free-vars
tags:
  - quality
  - lang
---

# Kernel free vars duplicated

## Report

Found by quality review 29 (range 0c489c2c..da5a38e0).

420ff9e0 added `Frees` and `Terms::frees` (`crates/adtb-lang/src/kernel.rs:312`, `:423-443`), a memo of each term's free variables as `HashMap<Tm, Rc<BTreeSet<Var>>>`. `crates/adtb/src/plan/terms.rs:206-222` `Facts::free` already memoises the same map over the same `Terms`, and the normaliser keeps a third, counting variant (`crates/adtb-lang/src/norm.rs:454`).

The two disagree on scope. `Facts::free` removes a binder only from the children it scopes (`bound(kind, n)`); `Terms::frees` removes every variable `t` binds from the union of all children, so for `Let(x, e, b)` or `For(x, e, b)` an `x` free in `e` is dropped. That is correct only while every variable has one binding site; a term violating it makes `renamed` share a subterm it must copy.

Fix: one memoised free-variable function in adtb-lang's kernel, scoped per child as `Facts::free` does, used by `Terms::renamed` and by `plan::Facts`.
