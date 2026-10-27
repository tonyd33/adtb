---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 866
summary: "derived.md 6.1 calls the retention window a setting in catalog.md 10, which lists none"
closed: 2026-10-27
tags:
  - spec
---

# Feature: state the retention window setting

## Report

`derived.md` section 6.1 says the retention window is a database setting in `catalog.md` section 10; section 10.2 lists no such setting.

Ruling: the retention window is a file setting in `catalog.md` section 10.2, a duration, default zero. At zero the engine keeps no history beyond what running snapshots see, and every `at` block outside a running snapshot is refused with `HistoryUnavailable`. Changing it applies to versions vacuum has not yet reclaimed.

## 2026-10-27: done

`catalog.md` §10.2 lists `RetentionWindow { length: Duration }`, default `0s`, constructor id 469 (§4.6 and the unassigned range updated, engine field ids now end at 563); `derived.md` §6.1 links it. The engine reads it at open and on `set setting`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the hcat track. What stays open is in `history-follow-ups`, `history-across-rewrites` and `heap-directory-interior-holes`.
