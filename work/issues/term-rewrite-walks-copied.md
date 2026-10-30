---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "four planner and delta walks repeat the same rebuild of a term from rewritten children, and subst and rename do one job twice"
tags:
  - quality
  - engine
---

# Term rewrite walks are copied across the planner and delta compiler

## Report

Moving `with_children` to the kernel left its callers repeating one body: read children, map them, return `t` when unchanged, otherwise `add(kind.with_children(&new), ty)`, memoised.

- `replace`: `crates/adtb/src/plan/lower/group.rs:254` and `crates/adtb/src/write/delta/mod.rs:1061` are the same function over `self.t` and `self.terms`, except that the first also memoises a mapped term.
- `rename`: `crates/adtb/src/plan/lower/split.rs:214` is `subst` (`crates/adtb/src/write/delta/mod.rs:1081`) with a one-entry map.
- `crates/adtb/src/plan/lower/mod.rs:546` `specialise` and `crates/adtb/src/write/fresh.rs:73` end in the same rebuild.

Fix: a kernel `Terms` method that rebuilds a term from a child rewrite, returning the term unchanged when no child changed, and one `replace` and one variable substitution on `Terms`. `plan` and `write` then call those. Check that each caller's `add` wrapper adds nothing but the arena call first.
