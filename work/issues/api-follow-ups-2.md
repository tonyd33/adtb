---
created-at: 2026-10-26
status: open
kind: bug
priority: 782
summary: "SchemaRefused.operator and Serialization.on are empty, sys::conflicts is never written, and deadline, limit, cancel, assert and trace options do not run"
---

# Api follow ups 2

## Report

## Report

Left by the API follow-ups track:
- `SchemaRefused.operator` is empty: `catalog::Error::Refused` holds only strings, so the catalog must map refusal spans back to operators.
- `Serialization.on` is empty and nothing is written to `sys::conflicts`: the serializer does not record which heap each rw-edge came from.
- `deadline` and `limit` are refused as Invalid; `cancel`, `assert` and `trace` are absent, since `CancelToken`, `Principal` and `TraceContext` do not exist.
- A failed result-shape check in `Prepared::results` ends an interactive transaction.
- `Violated.report` is untyped, and `Locked` never carries a pid.
- An in-thread deferrable begin while the same thread holds an open read-write transaction waits forever.
