---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "Catalog::emptied decides disable and remove from the literal operator, so a revert of an enable on a constructor that holds rows is not refused"
closed: 2026-10-27
tags:
  - engine
---

# Bug: the disable and remove data checks match operator text

## Report

`Catalog::emptied` in `crates/adtb/src/catalog/mod.rs` refuses `disable` and `remove constructor` while the constructor relation holds rows, but finds them by the literal operator. A `revert` of an `enable` disables the constructor without being refused. Use the step-schema diff by catalog id that `Catalog::changes` in `catalog/readers.rs` uses for readers first.

## 2026-10-27: done

`Catalog::emptied` runs per step on the step's schemas: a constructor relation whose state goes to declared while it holds rows is refused, and a value constructor that a step disables, or drops with no map carrying it, is refused while a stored field holds it. Tests in `write_states.rs` cover a revert of an enable on a relation constructor with rows, and reverts that disable or remove a stored value constructor.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.
