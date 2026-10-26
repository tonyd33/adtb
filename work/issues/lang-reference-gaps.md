---
created-at: 2026-10-25
status: open
kind: feature
priority: 270
summary: "the reference interpreter cannot run file scans or texts with temp relations"
tags:
  - reference
---

# Feature: Reference evaluation of file scans and temp relations

## Report

The reference interpreter returns `NotYet` for a file scan, and its
fixture runs only single-statement texts over the base schema, so a
text with temp relations cannot run. Expected: CSV and NDJSON scans
evaluated as `docs/spec/interop.md` states (Parquet and Arrow need a
crate, so ask first), and multi-statement texts with temp relations.
