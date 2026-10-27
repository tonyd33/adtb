---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 270
summary: the reference interpreter cannot run file scans or texts with temp relations
closed: 2026-10-26
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

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in vwztzyqp. The reference scans and loads CSV and NDJSON files, with or without `rejects into`, through a `Files` source (`Disk` or `InMemory`); Parquet and Arrow still return `NotYet`. Multi-statement texts with temp relations run in `Fixture::text` and the conformance runner. New conformance part `conformance/interop/` covers file scans, loading and temp relations.
