---
created-at: 2026-11-16
status: open
kind: bug
priority: 170
summary: "a rebuild that ends the reindexing marker while another rebuild still allocates leaves the other's pages unfreed after a crash"
tags:
  - engine
  - class-not-atomic
---

# Two rebuilds share one reindexing marker

Found by the review of catalog-roots-changing (cadb3d57), by reading; not reproduced.

The catalog root's `changing` marker records one change in progress. When two index rebuild jobs run at once, the second's `begin_reindexing` joins the first's `Reindexing` marker. The first ends the marker when its own pages are all named, while the second may still be allocating. A crash then leaves the second job's allocated pages unnamed with no marker, so no open frees them until something frees unnamed pages.

Same-kind markers carry no identity, so `end` cannot tell whose change it ends.

Fix direction: the marker counts or names the rebuilds holding it, and ends only when the last one ends; or rebuilds serialize on the marker. A test: two rebuilds, the first ends, crash before the second names its pages, reopen, assert the second's pages are freed.

Defect class: Multi-step change not atomic under failure, crash or race.

## 2026-11-17

Class count 2026-11-17: unreachable on main. crates/adtb/src/api/reindex.rs takes `holding_off_vacuum` (exclusive) before `begin_reindexing` and holds it past `end_reindexing`, so rebuild jobs serialize; the other joiners run only at open. Latent: the serialization is another lock's side effect. Fix: `begin_reindexing` takes `&VacuumHold`, so the type states it.
