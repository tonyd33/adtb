---
created-at: 2026-11-10
status: open
kind: bug
priority: 450
summary: "the damage search's check that no read serves past a durable mark looks only at scans, so a lookup through a key index served past a durable heap or database mark passes"
tags:
  - quality
  - test
  - sim
  - class-weak-tests
---

# Bug: lookups are not checked against durable marks

## Report

`durable_marks_unmet` (crates/adtb-sim/tests/storage/damage_search/judge.rs:1108) iterates only `f.case.observed.reads`. `observed.lookups`, which read through the key index, are never checked against a mark durable before the consumers ran, and `judge_read` accepts a lookup's allowed rows whatever the marks are. The violation's name, a read serving past a durable mark, claims every read.

Fix direction: iterate `o.lookups` too, a mark on the key index or its heap or the database refusing the lookup.

Found in the quality review of `reviewed..main` at 7bf40103.
