---
created-at: 2026-10-26
status: open
kind: feature
priority: 772
summary: "five new sim test binaries carry dead_code expectations whose reason names the symptom, caused by including support modules by path"
tags:
  - quality
  - sim
  - test
---

# Feature: sim test support as a library, without per-binary dead_code expectations

## Report

`crates/adtb-sim/tests/api.rs:4`, `commands.rs:4`, `derived_index.rs:5`, `files.rs:4` and `oplog.rs:4` each gain `#[expect(dead_code, reason = "this binary uses part of the API helpers")]`. The reason states the symptom, not what would remove it. The cause is shared: `support/api.rs` is pulled into each test binary by `#[path]`, and dead code is judged per binary. About 13 older binaries carry the same pattern for the device helpers and the planner fixture.

Fix: move the support modules into a library target (a `sim` module of adtb-testkit or a small test support crate), whose public items are not linted as dead, and delete every one of these expectations.
