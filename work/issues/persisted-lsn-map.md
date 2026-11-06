---
created-at: 2026-11-05
status: open
kind: bug
priority: 330
summary: lost writes whose last log record is recycled are undetectable; a persisted per-page LSN map folded before recycling keeps the evidence
tags:
  - class-damage-unreported
---

# Persist each page's last LSN before its log is recycled

## Report

## 2026-11-05

Ruled: storage/readin-gate/persisted-map with backup-retention/persist-before-recycle and map-lost-write/header-lsn. Before a segment is recycled, the last LSN of each page it names is folded into a persisted per-page map; read-in checks a page against the open map (retained log) or else the persisted map. The checkpoint record carries each map page LSN, so a lost write of the map itself is caught by its parent. Work at recycle time only. Mechanism of the damage class beside damage-marked-at-detection and damage-readin-gate. Lands after readin-gate changes 1 and 2.
