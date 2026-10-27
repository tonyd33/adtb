---
created-at: 2026-10-26
status: open
kind: bug
priority: 699
summary: "a read inside an arm of a top-level match still drops conditions on the arm variable, so such a constraint is refused as unbounded"
tags:
  - engine
  - constraints
---

# Bug: Admission drops conditions on a top-level match arm

## Report

A read inside an arm of a top-level `match` (a case at the query's top, not inside an expression) still drops conditions on the arm variable, because `top()` in `crates/adtb/src/write/delta/mod.rs` does not record arms. Such a constraint is refused as unbounded, though each arm is index-bounded on its own the same way `admission-path-forms` fixed paths inside nested match arms.