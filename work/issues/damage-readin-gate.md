---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 360
summary: "a page read in is checked against the retained log only in recovery, verify, scrub and the bounded evicted table, so a stale page is served as data and backup copies it as sound"
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# Gate every read-in on the retained log's last record for the page

## Report

## 2026-11-05

From the damage-reporting audit (work/briefs/audit-damage-reported-r1.md, reasoned from code at a8c1653a, not reproduced). `Pager::resident` and `lost_write` (storage/pager.rs:675-717) check only `past_log_end` and the evicted table. A page whose evicted slot was displaced, or any page after an open, is served stale; vacuum prunes and frees from it, a stale directory leaf lets writers use a freed page, dump reads old rows, and backup (backup.rs:816-871) copies it as sound with log only from redo, so the restored file can never detect it. Direction: `lost_write` falls back to the retained log's last LSN per page (verify's `read_log` builds that map; nothing recycles segments). This changes storage.md §3 and §10.3 (the lost-write row becomes every read-in while the retained log holds the page's record) and costs a log scan at open or a persisted per-page map: a design question through the space before code. Defect class: Damage reported as nothing.

## 2026-11-05 ruled

Adopted storage/damage-mark/damage-log (with derived-order/map-under-ser, record-full/overflow-database) and storage/readin-gate/open-map plus recycle (open-scan-cost/bound-by-recycle, backup-retention/live). Research: work/research/notes/damage-mechanism.md. Order: measure read_log per MiB first; if material, recycle lands with or before open-map; then damage-log. Briefs must carry: the map built by the coverage-checked scan, failing closed as LogUnproven; Unreadable never recorded; records kept in memory on a file marked whole; recovery records durable before clear_tail, with a crash search over that window; the map updated on every append (write-path cost named); every damage call site traced with its lock order and a scoped review; the holders walk under ser bounded. Recycle limits: never at or past redo, never a segment a running backup copies, never an unarchived one when an archive is set; the settle residual grows.

## 2026-11-06: change 2 landed

`fix: check every page read in against its expected LSN`. A page read in from the file is refused Stale when its LSN is below the expected LSN: the retained log's last record of the page, else the persisted map's entry, raised by every page record logged since the open; a blank page with an expected LSN is `Stale { found: 0 }`. One gate at `Pager::resident` through `lost_write`; `EvictedLsns` is deleted. `lsnmap::Gate` is `Recovering`, `Armed(Expected)` or `Off`: recovery arms it as its last step, so the bitmap, marks and map checks after it read through the armed gate, and no read path exists before. `Expected` holds the map's entries densely (8 bytes per page of the file) with an overlay for pages logged past their entry. A gap, unreadable block or undecodable batch before the end recovery kept, or a refused map page, holds Database quarantined for the open (`Off`) and writes no mark; held marks live apart and `change_marks` writes nothing while one is held. Only the commit-log settle (`mvcc::clog::Settling`) can read or rewrite a stale page, as a chained change, and `read_stale` refuses a page past the expected LSN. A damage during recovery drops the page's dirty entry, a refusal rather than a panic.

Tests: lost writes to a heap page, a directory leaf (stale-leaf-names-freed-page reproduced with the gate off), a page backed up after a reopen, a recycled page through the map and a page past its map entry are refused on first read; a log gap quarantines for the open and marks nothing; an open that cannot settle a status page writes no mark; the clog_lost sweeps revert every page an insert changes, the commit-log root included, and every page an allocating insert and a mark change touch, the bitmap and marks pages included, each refused on its next read. Changed expectations: lost_bitmap zeroed and cut cases are Stale; truncated_open's zeroed roles page is StalePage; 14 script timestamps and one clog crash loop round count moved with the open's longer simulated time. Review r2: blocker B1 (the bitmap and marks pages read before the gate was armed; a lost bitmap write let an insert reuse a heap page of committed rows) fixed by the gate's states; delta review blocker D1 (a chain break after a replay panicked the open) fixed. Not changed: `replace_contents` writes the marks page outside the held rule, reachable only by reindex, which a quarantined database refuses. Filed: damaged-dirty-entry-outlives-free. `check --changed` on 36fcc442: suite 3457 passed, campaign 48 passed.

## 2026-11-06: resolved
