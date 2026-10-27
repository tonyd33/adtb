---
created-at: 2026-10-27
status: open
kind: feature
priority: 832
summary: "in-process results are materialised, feeds and notices do not exist, encryption and format upgrade are missing, and several open settings are accepted and ignored"
tags:
  - engine
---

# Feature: embedded open follow-ups

## Report

From `docs/spec/embedding.md`, not yet built:

- Results are materialised before the first batch is returned, not streamed.
- Feeds and notices in process (sections 4.4 and 4.5).
- Encryption and `KeySource`, `FormatTooNew`, and `finalize_upgrade`.
- Open-time `temp_dir`, `archive` and `on_metrics`, and defaults derived from the host.
- `priority`, `trace` and `Quorum` are accepted and ignored.

## 2026-10-27: host error text

`HostError(String)` is public, but `exec/host.rs` discards its text and reports `Fault::FunctionFailed`, whose spec value has no detail. Ruling: keep the text in the API and route it to the trace and metrics hooks once `on_metrics` exists; until then, keep it in the engine's internal error log.
