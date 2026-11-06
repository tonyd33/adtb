---
created-at: 2026-11-05
status: open
kind: bug
priority: 340
summary: "recovery's Stale and Checksum pages are marked only when Engine::on fails; on a successful open they stay in memory and the next open serves them"
tags:
  - class-damage-unreported
---

# Recovery-found page damage is not marked on a successful open

## Report

## 2026-11-05

From the damage-reporting audit, confirmed in code: api/mod.rs `Engine::open` calls `mark_open_damage` only inside `Engine::on(...).or_else`. When the open succeeds, damage recovery recorded through `Pager::damage` lives only in the in-memory map, and recovery's closing checkpoint moves redo past the chain evidence, so after a reopen the page reads as valid until a verify over the full retained log finds it. storage.md §7 says recovery quarantines it. Direction: map and mark recovery's damage on every open. Test: a lost write recovery detects, open, close, reopen, read the page's relation: refused Quarantined. Defect class: Damage reported as nothing.
