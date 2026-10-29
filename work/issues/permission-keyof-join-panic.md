---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 831
summary: "the permission analyzer evaluates a composite KeyOf to rows, so joining it with any other Key value panics"
branch: permission-fixes
tags:
  - quality
  - lang
---

# The permission analyzer panics joining a composite KeyOf with another Key

## Report

`crates/adtb-lang/src/permission/eval.rs:55` evaluates `Kind::KeyOf(_, key)` to the key operand's value. For a composite key that operand is a `Kind::Record` (`crates/adtb-lang/src/check/expr.rs` about 1243-1258), so the value has shape `Shape::Rows`. Every other value of type `Key` (a stored Key field, a parameter, `unknown(Ty::Key)`, `Kind::Key`) is `Shape::Scalar`. Joining the two reaches `panic!("joined permission values have compatible checked types")` in `Value::join` (`crates/adtb-lang/src/permission/value.rs:240`).

Repro (from the code, not run): schema `type P = { a: I64, b: I64 }`, `relation ps : P key (a, b)`, `type L = { id: I64, target: Key ps }`, `relation ls : L key id`; run `permission::adhoc` on `{ps.key(a: 1, b: 2)} union (from l in ls select l.target)`.

Latent: no crate outside adtb-lang calls `permission::` yet.

Fix: make `KeyOf` return a scalar that carries the key operand's reads, as `Kind::Key` does. Add the repro as a test.
