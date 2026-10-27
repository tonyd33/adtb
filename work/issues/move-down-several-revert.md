---
created-at: 2026-10-27
status: open
kind: bug
priority: 796
summary: "reverting a move down to several constructors asks for fill arms for the listed ones after the first and drops their values"
tags:
  - lang
---

# Bug: reverting a move down to several constructors loses values

## Report

After `move field ... down to (Card | Invoice)`, the revert requires fill arms for `Invoice` and the other listed constructors after the first, and replaces their values with the fill. `evolution.md` declares the loss only for the constructors not listed; the listed ones keep their values when the field moves back up. The ignored test `reverting_a_move_down_to_two_constructors_keeps_their_values_and_fills_the_others` in `crates/adtb-sim/tests/oplog.rs` states it. Remove the kind from the `UNREVERTED` skip list in `crates/adtb-testkit/tests/differential.rs` once it passes.
