---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "Catalog::emptied decides disable and remove from the literal operator, so a revert of an enable on a constructor that holds rows is not refused"
tags:
  - engine
---

# Bug: the disable and remove data checks match operator text

## Report

`Catalog::emptied` in `crates/adtb/src/catalog/mod.rs` refuses `disable` and `remove constructor` while the constructor relation holds rows, but finds them by the literal operator. A `revert` of an `enable` disables the constructor without being refused. Use the step-schema diff by catalog id that `Catalog::changes` in `catalog/readers.rs` uses for readers first.
