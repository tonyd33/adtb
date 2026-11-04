---
created-at: 2026-11-04
status: open
kind: bug
priority: 560
summary: "A background vacuum yields the whole run to any vacuum-lock waiter and restarts from scratch, so frequent verify calls can keep a large relation from ever finishing"
---

# Background vacuum restarts after every yield

## Report

The background vacuum yields when anyone waits for the vacuum lock: `verify` (once per index), `set setting`, `vacuum r`. The yield cancels the whole run, and the relation's next run starts from the first page. A relation big enough that a run outlasts the gap between `verify` calls is never finished in the background. The yield itself is correct and tested; the cost is the lost progress.

Found in the review of background vacuum V3, 2026-11-04.

## Proposal

Resume a yielded run where it stopped: keep the scanned position and the dead set gathered so far, provided the horizon it was gathered under still holds. Or let a waiter that only reads (verify) wait for the current step instead of cancelling the run.
