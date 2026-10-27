---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "file settings are read by matching constructor name spellings and field position, and a row of any other shape is ignored silently"
tags:
  - quality
  - engine
---

# File settings are read by constructor name spelling and field position

## Report

`Settings::set` (`crates/adtb/src/api/settings.rs:63-75`) dispatches on the strings `"DefaultLease"`, `"LeaseGrace"` and `"RetryLimit"` and reads `fields.get(1)`. Its `_ => {}` arm silently drops a setting whose constructor is renamed or whose value field moves, and a negative `RetryLimit` becomes 0 through `unwrap_or(0)`.

Fix: resolve the settings sum's constructor ids once from the schema, and refuse an unexpected shape or a negative limit as corrupt.
