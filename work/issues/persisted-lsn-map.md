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

## 2026-11-05 format ruled

Page kind LsnMap = 12, a body of u64 LSNs indexed by pid offset; one root map page lists each child map page with its LSN (children checked against it as a parent pointer); Record::Checkpoint carries the root pid and LSN, so the root is checked by the checkpoint; an old checkpoint without it reads as no map. A map page found stale quarantines the whole database (a page no heap or index holds, storage.md 8). A fold covers pids below the file page count and drops records for pids past it. The fold runs inside recycle_log: pin, scan pid and LSN only with no lock, logged map changes taking the max, flush and checkpoint naming the map, then remove segments oldest first. Built in readin-gate change 1 (slot-44).
