---
created-at: 2026-11-10
status: open
kind: bug
priority: 800
summary: "Testing::retired, a new test read of the pages held as retired, is missing from testing.md"
tags:
  - quality
  - test
  - class-none
---

# Bug: Testing::retired is undocumented

## Report

`retired()` (crates/adtb/src/testing/control.rs:52), used by crates/adtb-sim/tests/storage/damage_search/world.rs:587, is missing from docs/design/testing.md's table of object reads and its list of calls, the contract the sim relies on.

Fix direction: add the row stating what it returns.

Found in the quality review of `reviewed..main` at 7bf40103.
