---
created-at: 2026-11-10
status: open
kind: bug
priority: 390
summary: "verify and scrub report no StalePage for a lost write whose page's record lies before a log gap in the retained log"
tags:
  - class-damage-unreported
---

# verify misses a stale page whose record lies before a log gap

## Report

## 2026-11-10

Found by the damage search's fault pairs tier (damage-search-fault-pairs): a lost write of a page, then a lost log write after that page's record. `verify` reports the `Gap` and quarantines the database whole, but names no `StalePage` for the page, and scrub names nothing. Cases:

- damage profile, seed 2: Lost at write-out 645, then LogLost of log write 83 (at 647). Findings: `page 572: Checksum`, `log at 6534144: Gap`, the whole-database row; page 3 is missing.
- deep profile, seed 1: Lost at 612, then LogLost of log writes 33, 40, 47 (at 633). Verify and scrub miss page 253.
- deep profile, seed 3: Lost at 711, then LogLost of log write 53 (at 712). Scrub misses the page.

storage.md §7 states that `verify` checks the chain over the whole retained log and that each page's LSN is that of its last record there. A page whose LSN is below a record the retained log holds for it before the gap is stale whatever the gap hides. The whole-database mark refuses every read, so no stale row is served; the finding is still owed.

Direction: find where verify's and scrub's per-page LSN check stops at a gap; check each page against the last record read for it before the gap. The three cases are pinned in the search's `KNOWN` under this stem.

Defect class: Damage reported as nothing.
