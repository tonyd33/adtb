---
created-at: 2026-10-27
status: open
kind: bug
priority: 790
summary: "a generated Tag enum orders its variants by name, so it decodes a tag whose sum is not declared in name order as the wrong constructor"
tags:
  - quality
  - lang
---

# A generated Tag enum decodes tags as the wrong constructor

## Report

`Emitter::tag` (`crates/adtb-codegen/src/emit.rs:204-217`) sorts the variants of a declared sum's tag type by name, then `enum_items` (`emit.rs:501`) maps variant position `i` to constructor index `i` in both `to_value` and `from_value`. `docs/spec/model.md:215` and `docs/spec/queries.md:735` give `Tag T` the constructors of `T` "of the same names in the same order", and the engine (`textual/form.rs:130`, `exec/types.rs:41`) uses declaration order.

So for `type Status = Open | Closed`, `StatusTag::from_value` reads `tag(Open)` (constructor 0) as `StatusTag::Closed`, and `to_value(StatusTag::Closed)` writes Open.

No test sees it: every fixture sum in `crates/adtb-codegen-tests/fixtures/shop` (`Tier = Free | Paid`, `Payment = Card | Invoice`) is declared in name order, and no fixture callable takes or returns a `Tag`.

Repro: a bundle query `() -> Set (Tag Status) = from s in ss select tag(s)` over one `Open` row decodes as `StatusTag::Closed`.

Fix: drop the sort and emit variants in declaration order, as declared sums already do; fix the doc comment. Add a fixture sum declared out of name order with a callable over its `Tag` to `crates/adtb-codegen-tests`.
