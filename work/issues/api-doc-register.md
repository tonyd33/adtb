---
created-at: 2026-10-27
status: open
kind: bug
priority: 865
summary: "new doc comments in api, evolve, parse, the probe and the testkit are in the third person instead of the imperative"
tags: [quality]
---

# New doc comments are written in the third person

## Report

The house rule is doc comments in the imperative, third person only for "Returns". This range adds third-person ones, among them:

- `crates/adtb/src/api/error.rs:344`: "Writes the error as messages..."
- `crates/adtb/src/api/txn.rs`, `AdHocTransaction::run`: "Runs the items of `p`..."
- `crates/adtb-lang/src/evolve/grant.rs:40` "Adds", `:68` "Resolves"
- `crates/adtb-lang/src/evolve/revert.rs:27` "Adds module"
- `crates/adtb-lang/src/parse/mod.rs:131` "Parses"
- the operator docs in `crates/adtb-testkit/src/cases/walk.rs`, and "Runs" and "Sweeps" in `crates/adtb-testkit/tests/differential.rs`
- `crates/adtb-probe/src/main.rs:138` "Checks the schema ... and returns", and `crates/adtb-probe/tests/probe.rs:13` "Runs the probe ..."

Also `crates/adtb-probe/src/main.rs:43` puts one doc comment above a group of consts, so it attaches to `SCHEMA` only.

Older code does the same (`evolve/mod.rs:688` "Applies one operator..."), and `work/tools/house-check` does not catch it.

Fix: rewrite these in the imperative, and have `house-check` flag an added `///` whose first word is a third-person verb other than "Returns".

## 2026-10-27: ruling

Sweep the third-person doc comments this range added to the imperative, and add a `house-check` rule that flags an added doc comment whose first word is a third-person verb other than "Returns". Older code is converted only when a change touches it.
