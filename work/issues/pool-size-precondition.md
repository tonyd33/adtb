---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 409
summary: nothing enforces that all threads together latch fewer pages than the pool has frames; running short stalls
branch: pager
tags:
  - engine
---

# Bug: The pool-size precondition is unenforced

## Report

`docs/design/storage.md` §2.3 requires that the pages latched at once
by all threads together are fewer than the pool's frames. A B-link
operation latches up to four pages plus those `allocate` takes, so the
pool needs more than about four frames per thread. Running short stalls
in `make_room`. Expected: the engine sizes its thread count against the
pool at open, and `make_room` reports a stall instead of spinning.
