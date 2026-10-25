---
created-at: 2026-10-25
status: open
kind: feature
priority: 190
summary: "the front end has no fuzzer, and milestone 1 exits only after one runs a fixed budget with no crash"
tags: [lang, test]
---

# Feature: front-end fuzzer

## Report

Milestone 1 of `docs/plan.md` tests the front end by fuzzing from day
one (`docs/design/oracle.md` §5.6), and exits when the fuzzer has run
for a fixed budget with no crash. No fuzzer exists.
