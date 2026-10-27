---
created-at: 2026-10-26
status: open
kind: feature
priority: 785
summary: "the engine has no ReadFile and refuses temp declarations, so conformance/interop 03 to 05 fail on it"
tags:
  - engine
---

# Feature: File scans and temp relations in the engine

## Report

The reference now scans CSV and NDJSON files, loads them with rejects, and runs texts that declare temp relations. The engine has no file scan and refuses temp declarations, so `conformance/interop/03-file-scans`, `04-loading` and `05-temp-relations` are on `engine-failing.txt`.
