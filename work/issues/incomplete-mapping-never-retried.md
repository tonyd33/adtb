---
created-at: 2026-11-06
status: open
kind: bug
priority: 450
summary: "a damage record whose mapping came back incomplete is skipped by commits and retried only by a failing call, verify, scrub or open, so on a long-running server it can stay unmapped after its cause clears"
tags:
  - class-damage-unreported
---

# An incompletely mapped damage record is never retried on a quiet server

## Report

## 2026-11-06

From the damage-log change 2 delta review (D2). A record whose mapping came back incomplete (an unreadable page stopped the holders walk) is skipped by commits under `ser` and retried only by a failing call, verify, scrub or open. On a long-running server it can stay unmapped after the cause clears, and commits that write the damaged object but not the damaged page are admitted meanwhile. Nothing is lost: the damaged page itself refuses every read. Direction: a periodic retry outside the commit lock, for example on the background worker's tick.
