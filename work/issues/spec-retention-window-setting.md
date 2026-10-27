---
created-at: 2026-10-27
status: open
kind: feature
priority: 866
summary: "derived.md 6.1 calls the retention window a setting in catalog.md 10, which lists none"
tags:
  - spec
---

# Feature: state the retention window setting

## Report

`derived.md` section 6.1 says the retention window is a database setting in `catalog.md` section 10; section 10.2 lists no such setting.

Ruling: the retention window is a file setting in `catalog.md` section 10.2, a duration, default zero. At zero the engine keeps no history beyond what running snapshots see, and every `at` block outside a running snapshot is refused with `HistoryUnavailable`. Changing it applies to versions vacuum has not yet reclaimed.
