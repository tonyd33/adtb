---
created-at: 2026-10-26
status: open
kind: feature
priority: 776
summary: "the soundness test copies adtb-conformance's private schema builder line for line"
tags:
  - quality
  - test
---

# Feature: the soundness test reuses adtb-conformance's schema builder

## Report

`conformance_schema` (`crates/adtb-testkit/tests/soundness.rs:524-546`) copies adtb-conformance's private `modules` (`crates/adtb-conformance/src/lib.rs:230-242`) line for line, and `schema` (lib.rs:207-226) apart from the fixture it returns.

Fix: export the schema builder from adtb-conformance, which soundness already depends on, and delete the copy.
