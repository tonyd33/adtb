---
created-at: 2026-11-05
status: open
kind: bug
priority: 360
summary: "a page read in is checked against the retained log only in recovery, verify, scrub and the bounded evicted table, so a stale page is served as data and backup copies it as sound"
tags:
  - class-damage-unreported
---

# Gate every read-in on the retained log's last record for the page

## Report

## 2026-11-05

From the damage-reporting audit (work/briefs/audit-damage-reported-r1.md, reasoned from code at a8c1653a, not reproduced). `Pager::resident` and `lost_write` (storage/pager.rs:675-717) check only `past_log_end` and the evicted table. A page whose evicted slot was displaced, or any page after an open, is served stale; vacuum prunes and frees from it, a stale directory leaf lets writers use a freed page, dump reads old rows, and backup (backup.rs:816-871) copies it as sound with log only from redo, so the restored file can never detect it. Direction: `lost_write` falls back to the retained log's last LSN per page (verify's `read_log` builds that map; nothing recycles segments). This changes storage.md §3 and §10.3 (the lost-write row becomes every read-in while the retained log holds the page's record) and costs a log scan at open or a persisted per-page map: a design question through the space before code. Defect class: Damage reported as nothing.

## 2026-11-05 ruled

Adopted storage/damage-mark/damage-log (with derived-order/map-under-ser, record-full/overflow-database) and storage/readin-gate/open-map plus recycle (open-scan-cost/bound-by-recycle, backup-retention/live). Research: work/research/notes/damage-mechanism.md. Order: measure read_log per MiB first; if material, recycle lands with or before open-map; then damage-log. Briefs must carry: the map built by the coverage-checked scan, failing closed as LogUnproven; Unreadable never recorded; records kept in memory on a file marked whole; recovery records durable before clear_tail, with a crash search over that window; the map updated on every append (write-path cost named); every damage call site traced with its lock order and a scoped review; the holders walk under ser bounded. Recycle limits: never at or past redo, never a segment a running backup copies, never an unarchived one when an archive is set; the settle residual grows.
