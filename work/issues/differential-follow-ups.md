---
created-at: 2026-10-26
status: open
kind: feature
priority: 788
summary: "the differential tests have no compiled versus interpreted toggle, schema walks register no bundles, shrinking keeps unused relations and walk failures are not shrunk"
tags:
  - test
---

# Feature: Differential test follow ups

## Report

Left by the differential strength track:
- `exec` has no switch between compiled and interpreted execution, so the plan toggles cannot compare them.
- Schema walks do not register bundles or recheck registered queries after a migration.
- Shrinking does not drop relations the failing query does not read, and a failing walk is not shrunk at all.
- Opening the engine is about 94% of a differential case's time; see `test-suite-time`.
