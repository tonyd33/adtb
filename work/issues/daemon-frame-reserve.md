---
created-at: 2026-11-06
status: open
kind: bug
priority: 500
summary: the pool's frame reserve does not count the latches of the checkpointer, vacuum and scrub daemons
tags:
  - class-none
---

# Bug: the frame reserve misses the daemons' latches

## Report

Found by the periodic-checkpoint change 1 review (2026-11-06). storage.md sizes the pool's reserve of frames by the latches a call may hold at once (the B-link operation's four and a bitmap page); the background checkpointer's write-out holds a page latch, and the background vacuum and scrub daemons hold latches too, none counted. With every call thread at its bound and the daemons running, a page-in can find no frame and stall until the pool's stall bound panics.

Expected: the reserve counts each daemon's latches, or the daemons take frames from a reserve of their own; a test with every call thread at its bound and each daemon running completes.
