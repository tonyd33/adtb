---
created-at: 2026-11-04
status: resolved
closed: 2026-11-04
kind: bug
priority: 360
summary: A verify that runs while a write to a damaged page is not yet durable reports nothing for that page
---

# Verify misses damage behind a non-durable write

## Report

After a writer rewrites a page whose LSN was past the log end, a `verify(None)` run before that write's record is durable returns no findings: the pool LSN is past the end the run checks against. A second `verify` once the record is durable reports `StalePage` and `Unlogged`. So whether damage is reported depends on when the last write to the page became durable.

Found in background vacuum V3, delta round 2, 2026-11-04.

## Proposal

Have `verify` judge a page against the log end it observes durable when the page is read, or flush to the page's LSN before judging it. Fixing page-past-log-end-accepted removes the path that produced this case; check whether any other remains.

## 2026-11-04: closed by page-past-log-end-accepted

A writer can no longer rewrite a page whose file copy is past the log end, so the path that produced this case is gone. The remaining timing dependence is verify's excuse for pages written after it starts; a rewrite of an older stale page after the check starts is reported by the next run from the record chain. Traced, not tested.
