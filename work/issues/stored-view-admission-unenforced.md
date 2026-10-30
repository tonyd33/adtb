---
created-at: 2026-10-29
status: open
kind: bug
priority: 515
summary: "store admits a view with no index-bounded maintenance plan, which derived.md 2.2 refuses"
---

# Stored view admission unenforced

## Report

Found by the view-panic track. A view counting over an unindexed `toys.label` stored with no refusal; reads evaluate the body. Milestone 12 owns derived features; confirm the plan there before building.
