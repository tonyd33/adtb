---
created-at: 2026-11-07
status: resolved
kind: bug
priority: 430
summary: "verify reports a chain break of a page settled, freed and reused, since the break is never cleared at a chain start"
closed: 2026-11-07
tags:
  - class-damage-unreported
---

# verify reports a chain break of a page settled, freed and reused, since the break is never cleared at a chain start

## Report

## 2026-11-07: fixed

Fixed by `feat: start a page's log chain at allocation and log every free` (page-incarnation-identity, M1). An allocation logs `Image { prev: UNCHAINED }`, a chain start, so a settled, freed and reused page's old break is not carried. Test: `incarnation::a_status_page_settled_freed_and_reused_is_reported_by_nothing`; main reported Stale.

## 2026-11-07: resolved
