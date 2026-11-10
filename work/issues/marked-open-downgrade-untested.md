---
created-at: 2026-11-10
status: open
kind: bug
priority: 600
summary: "no marked-open test drives a downgrade refused under a whole-database mark, though the range removed the writable gate on registered::downgrade"
tags:
  - quality
  - engine
  - test
  - class-weak-tests
---

# Bug: DowngradeRefused untested under a mark

## Report

The range removed the `if writable` gate on `registered::downgrade` (crates/adtb/src/api/mod.rs:1068) and lists `DowngradeRefused` in `refuses_the_program`. crates/adtb/src/api/marked_open_tests.rs covers `Diverged`, host functions and `MigrationRefused` under both mark kinds, and no downgrade.

Fix direction: add a downgrade case to the `MARKS` loop, asserting the refusal's value and reason.

Found in the quality review of `reviewed..main` at 7bf40103.
