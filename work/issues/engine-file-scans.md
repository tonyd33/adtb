---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 785
summary: "the engine has no ReadFile and refuses temp declarations, so conformance/interop 03 to 05 fail on it"
closed: 2026-10-26
tags:
  - engine
---

# Feature: File scans and temp relations in the engine

## Report

The reference now scans CSV and NDJSON files, loads them with rejects, and runs texts that declare temp relations. The engine has no file scan and refuses temp declarations, so `conformance/interop/03-file-scans`, `04-loading` and `05-temp-relations` are on `engine-failing.txt`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in rqxztvot. The readers moved to a new crate, `adtb-file`, shared by the engine and the reference. The engine binds each file scan and temp relation read to a variable at the start of a statement, keeps temp rows and file checksums per transaction, and aborts with `FileChanged` when a scanned file changes. `conformance/interop/03` to `05` pass on the engine.
