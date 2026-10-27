---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "Catalog::replay and Catalog::changes skip a migration step whose schema does not build, so a bad step panics later instead of being refused"
tags:
  - engine
---

# Bug: replay skips a step whose schema does not build

## Report

`Catalog::replay` and `Catalog::changes` in `crates/adtb/src/catalog/` run `let Ok(schema) = step.after.schema() else { continue; }`. A step whose schema does not build is skipped silently, and the rules after it run on a schema that does not match the data; a bad merge revert reached a panic at `change.rs:149` (`ctors[&cid]`) this way. A step whose schema does not build must refuse the migration, naming the operator, or be an internal error.
