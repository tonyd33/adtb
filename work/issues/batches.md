---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 710
summary: "no batch type: no Arrow-layout buffers, dense unions or the special shapes"
closed: 2026-10-25
tags:
  - engine
---

# Feature: Arrow-layout batches

## Report

Implement `docs/design/execution.md` §2: own Arrow-layout buffers in
`adtb`, dense unions for sums, and the three special shapes. No
prerequisites.

## 2026-10-25: resolved

Landed as `feat(batch): add Arrow-layout batches with dense unions and the special sum shapes`. The layout contract is `docs/design/execution.md` §2.
