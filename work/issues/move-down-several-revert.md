---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 796
summary: reverting a move down to several constructors asks for fill arms for the listed ones after the first and drops their values
closed: 2026-10-27
tags:
  - lang
---

# Bug: reverting a move down to several constructors loses values

## Report

After `move field ... down to (Card | Invoice)`, the revert requires fill arms for `Invoice` and the other listed constructors after the first, and replaces their values with the fill. `evolution.md` declares the loss only for the constructors not listed; the listed ones keep their values when the field moves back up. The ignored test `reverting_a_move_down_to_two_constructors_keeps_their_values_and_fills_the_others` in `crates/adtb-sim/tests/oplog.rs` states it. Remove the kind from the `UNREVERTED` skip list in `crates/adtb-testkit/tests/differential.rs` once it passes.

## 2026-10-27: done

The inverse of `move field T.f to T.(C | D)` moves the field of every listed constructor back to the header in one step, each keeping its value; the fill covers only the constructors not listed, and none is asked when every constructor was listed. `UNREVERTED` is gone. The oplog test runs, querying each constructor separately since a set result merges equal notes.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.
