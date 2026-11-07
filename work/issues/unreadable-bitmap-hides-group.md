---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 600
summary: "an unreadable bitmap page makes called_free true for its whole group, so verify and scrub report nothing for those pages that run"
closed: 2026-11-07
tags:
  - class-damage-unreported
---

# An unreadable bitmap hides its group's damage

## Report

## 2026-11-05

From the damage-reporting audit: storage/quarantine.rs `called_free` returns true on `Unreadable`, and verify's `check_page` returns early on it, so one transient I/O error on a bitmap page hides every page of its group for that run; only the bitmap page gets a Page Unreadable row. Direction: treat an unreadable bitmap as unknown and report each page's findings. Defect class: Damage reported as nothing.

## 2026-11-07: fixed

Fixed by `feat: start a page's log chain at allocation and log every free` (page-incarnation-identity, M1). `Pager::allocation(pid) -> Allocation { Allocated, Free, Unknown }` replaces `called_free`'s bool in verify and the holder walks; only Free excuses a page, a refused bitmap page gives Unknown. Test: `pager::verify_reports_the_damage_of_a_page_whose_bitmap_page_the_device_cannot_read`; seed 203 of the bit-rot test found it once the allocation schedule changed.

## 2026-11-07: resolved
