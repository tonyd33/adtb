---
created-at: 2026-11-06
status: open
kind: bug
priority: 480
summary: "a damaged dirty page's dirty-table entry outlives a free of the page and pins the redo point for the open"
tags:
  - class-none
  - engine
---

# Damaged dirty entry outlives a free

## Report

## Report

Since the read-in gate's change 2, `Pager::damage` of a dirty frame keeps the page's dirty-table entry so the checkpoint never passes a logged change nothing wrote. The entry stays for the rest of the open: it pins the checkpoint's redo point, so recycling stops and the log grows, and it outlives a later `free` of the page. Found by the change 2 review (N4). Decide when the entry may go: after the page is freed and the free is durable, or after a repair rewrites the page.
