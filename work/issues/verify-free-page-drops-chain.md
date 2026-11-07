---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 365
summary: "verify and scrub return no findings for a page the bitmap calls free, so a broken log chain recorded while the page was allocated is discarded"
closed: 2026-11-07
tags:
  - class-damage-unreported
---

# Verify drops a freed page's broken chain

## Report

## 2026-11-05

From the damage-reporting audit, confirmed in code: storage/verify.rs `check_page` returns `findings` when `called_free(pid)` (the loop and the line after it) before `stale` and `unlogged` are pushed. A stale heap page that vacuum prunes empty and frees (heap/handle.rs:853-907) loses its StalePage finding, which is the mechanism of vacuum-erases-lost-write. Direction: report a chain break the retained log proves for a page whatever the bitmap says now; narrow catalog.md §11.1's free-page exemption to the page's contents. Defect class: Damage reported as nothing.

## 2026-11-07: fixed

Fixed by `feat: start a page's log chain at allocation and log every free` (page-incarnation-identity, M1). Verify reports an unexcused chain break of a page the bitmap calls free, without refusing it; a break matching a Settled record of the same incarnation is excused. Test: `verify_reports_a_chain_break_of_a_page_freed_since` (storage unit tests); main found nothing.

## 2026-11-07: resolved
