---
created-at: 2026-11-05
status: open
kind: bug
priority: 365
summary: "verify and scrub return no findings for a page the bitmap calls free, so a broken log chain recorded while the page was allocated is discarded"
---

# Verify drops a freed page's broken chain

## Report

## 2026-11-05

From the damage-reporting audit, confirmed in code: storage/verify.rs `check_page` returns `findings` when `called_free(pid)` (the loop and the line after it) before `stale` and `unlogged` are pushed. A stale heap page that vacuum prunes empty and frees (heap/handle.rs:853-907) loses its StalePage finding, which is the mechanism of vacuum-erases-lost-write. Direction: report a chain break the retained log proves for a page whatever the bitmap says now; narrow catalog.md §11.1's free-page exemption to the page's contents. Defect class: Damage reported as nothing.
