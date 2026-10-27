---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "Catalog::replay and Catalog::changes skip a migration step whose schema does not build, so a bad step panics later instead of being refused"
closed: 2026-10-27
tags:
  - engine
---

# Bug: replay skips a step whose schema does not build

## Report

`Catalog::replay` and `Catalog::changes` in `crates/adtb/src/catalog/` run `let Ok(schema) = step.after.schema() else { continue; }`. A step whose schema does not build is skipped silently, and the rules after it run on a schema that does not match the data; a bad merge revert reached a panic at `change.rs:149` (`ctors[&cid]`) this way. A step whose schema does not build must refuse the migration, naming the operator, or be an internal error.

## 2026-10-27: done

`catalog/steps.rs` `built` judges each step for `Catalog::replay` and `Catalog::changes`. A step whose schema does not build and moves rows refuses the migration, naming the operator's first line. A step that moves no rows folds into the next step that builds: `drop relation labels` followed by `drop type Label` passes through such a schema legitimately. `replay` now also stops at the first step whose data map is refused, so later steps no longer run on data in the layout of an older schema.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the q799 track.
