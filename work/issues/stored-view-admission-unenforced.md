---
created-at: 2026-10-29
status: wontfix
kind: bug
priority: 515
summary: "store admits a view with no index-bounded maintenance plan, which derived.md 2.2 refuses"
closed: 2026-10-31
---

# Stored view admission unenforced

## Report

Found by the view-panic track. A view counting over an unindexed `toys.label` stored with no refusal; reads evaluate the body. Milestone 12 owns derived features; confirm the plan there before building.

## 2026-10-31: closed by triage

Duplicate of stored-views.

## 2026-10-31: wontfix
