---
created-at: 2026-11-05
status: open
kind: bug
priority: 600
summary: "an unreadable bitmap page makes called_free true for its whole group, so verify and scrub report nothing for those pages that run"
tags:
  - class-damage-unreported
---

# An unreadable bitmap hides its group's damage

## Report

## 2026-11-05

From the damage-reporting audit: storage/quarantine.rs `called_free` returns true on `Unreadable`, and verify's `check_page` returns early on it, so one transient I/O error on a bitmap page hides every page of its group for that run; only the bitmap page gets a Page Unreadable row. Direction: treat an unreadable bitmap as unknown and report each page's findings. Defect class: Damage reported as nothing.
