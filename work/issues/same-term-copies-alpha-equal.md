---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "the planner's same_term repeats kernel alpha_equal with an erased type test and a mapping checked in one direction only"
tags:
  - quality
  - engine
  - lang
---

# The planner's same_term repeats alpha_equal

## Report

`Planner::same_term` (`crates/adtb/src/plan/lower/probe.rs:242`) blanks children, collects variables with `with_vars`, compares kinds and recurses, which is `alpha_equal` (`crates/adtb-lang/src/kernel.rs:902`). The range rewrote it onto the kernel's `with_children` and `with_vars`, which leaves the two side by side in the same terms. They differ in three ways: `same_term` compares erased types, takes a starting variable map, and checks the map in one direction only. `alpha_equal` keeps `there` and `back`.

The one direction means two variables of `b` can map to one of `a`, so `same_term` can call `f(z, z)` the same as `f(x, y)` when `x` and `y` are binders of `b`. I found no reachable input: the caller seeds only the row variable (`probe.rs:234`) and binders in a normal arena are distinct. Not run; read only.

Fix: one kernel function taking the seed and a type comparison, `alpha_equal` being its call with an empty seed and exact types.
