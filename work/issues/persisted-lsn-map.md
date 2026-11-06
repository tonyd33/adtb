---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 330
summary: lost writes whose last log record is recycled are undetectable; a persisted per-page LSN map folded before recycling keeps the evidence
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# Persist each page's last LSN before its log is recycled

## Report

## 2026-11-05

Ruled: storage/readin-gate/persisted-map with backup-retention/persist-before-recycle and map-lost-write/header-lsn. Before a segment is recycled, the last LSN of each page it names is folded into a persisted per-page map; read-in checks a page against the open map (retained log) or else the persisted map. The checkpoint record carries each map page LSN, so a lost write of the map itself is caught by its parent. Work at recycle time only. Mechanism of the damage class beside damage-marked-at-detection and damage-readin-gate. Lands after readin-gate changes 1 and 2.

## 2026-11-05 format ruled

Page kind LsnMap = 12, a body of u64 LSNs indexed by pid offset; one root map page lists each child map page with its LSN (children checked against it as a parent pointer); Record::Checkpoint carries the root pid and LSN, so the root is checked by the checkpoint; an old checkpoint without it reads as no map. A map page found stale quarantines the whole database (a page no heap or index holds, storage.md 8). A fold covers pids below the file page count and drops records for pids past it. The fold runs inside recycle_log: pin, scan pid and LSN only with no lock, logged map changes taking the max, flush and checkpoint naming the map, then remove segments oldest first. Built in readin-gate change 1 (slot-44).

## 2026-11-06: landed with log recycling

`feat: fold page LSNs into a persisted map before recycling the log` (storage/lsnmap.rs). Recycling runs only at the open of an existing file, after every reader of the retained log, after one archive pass; never at close or on create. It removes segments wholly below the least of the durable checkpoint's redo, the start of this open's log (the session floor), every reader pin (`load_log`) and the archive's `archived_through()`. Before removal the last LSN of each page in those segments is folded (max) into a radix map of LsnMap pages; each map page is checked against its parent's recorded LSN, the root against the checkpoint or a later `MapRoot` log record whose root must have a record in its batch (else the open refuses LogUnproven); a fold creates each new page in the batch that links it, so a crash leaks nothing. verify, scrub and backup fall back to the map when the retained log has no record. A checksum-bad, wrong-kind or stale map page quarantines the database; an unreadable one is refused unmarked; `allocated()` fails rather than return a partial list. Review: no blocker; fixed an untested backup pin (dropped: the session floor covers it until the floor can move in-session, see periodic-checkpoint), a map-root race at growth, a pin leak on panic, map refusals swallowed, map pages freed through `allocated()`, an archive pass on a discarded create.

Residuals: the race fix at growth (`meta.map` set under the log lock) has no test, the sim cannot place a checkpoint in the window; a page unreadable once stays refused for the rest of the session. Next: damage-readin-gate (change 2, open-map: every read-in checked against the retained-log map, then this map).

## 2026-11-06: resolved
