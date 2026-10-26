---
created-at: 2026-10-25
status: open
kind: feature
priority: 470
summary: "the pager has no explicit close; its data file and log segments are released only when the runtime drops"
tags: [engine]
---

# Feature: close a pager

## Report

From the real runtime track. `Close` exists in the I/O interface, but the pager never calls it except for log segments `clear_tail` removes, since destructors must not block. Add an explicit close that flushes, closes the data file and every log segment, and state it in `docs/design/storage.md`.
