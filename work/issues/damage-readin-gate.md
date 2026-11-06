---
created-at: 2026-11-05
status: open
kind: bug
priority: 360
summary: "a page read in is checked against the retained log only in recovery, verify, scrub and the bounded evicted table, so a stale page is served as data and backup copies it as sound"
---

# Gate every read-in on the retained log's last record for the page

## Report

## 2026-11-05

From the damage-reporting audit (work/briefs/audit-damage-reported-r1.md, reasoned from code at a8c1653a, not reproduced). `Pager::resident` and `lost_write` (storage/pager.rs:675-717) check only `past_log_end` and the evicted table. A page whose evicted slot was displaced, or any page after an open, is served stale; vacuum prunes and frees from it, a stale directory leaf lets writers use a freed page, dump reads old rows, and backup (backup.rs:816-871) copies it as sound with log only from redo, so the restored file can never detect it. Direction: `lost_write` falls back to the retained log's last LSN per page (verify's `read_log` builds that map; nothing recycles segments). This changes storage.md §3 and §10.3 (the lost-write row becomes every read-in while the retained log holds the page's record) and costs a log scan at open or a persisted per-page map: a design question through the space before code. Defect class: Damage reported as nothing.
