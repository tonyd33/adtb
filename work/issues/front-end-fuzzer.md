---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 190
summary: "the front end has no fuzzer, and milestone 1 exits only after one runs a fixed budget with no crash"
closed: 2026-10-26
tags:
  - lang
  - test
---

# Feature: front-end fuzzer

## Report

Milestone 1 of `docs/plan.md` tests the front end by fuzzing from day
one (`docs/design/oracle.md` §5.6), and exits when the fuzzer has run
for a fixed budget with no crash. No fuzzer exists.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrvrlkrt.
The fuzzer is `crates/adtb-testkit/src/fuzz.rs`, run by `crates/adtb-testkit/tests/front_fuzz.rs` with a budget of 3000 cases; about 245k further seeds ran clean.
