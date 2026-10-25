---
title: How an index is rebuilt
pick: some
raised-by: [index/ctor-set]
options:
  online:
    title: Build a new index from sorted runs of each heap while writes continue, then swap
  offline:
    title: Rebuild with writes to the sum blocked
---
Needed after a check finds a bad index, or after a release changes key
encoding. A union index ranges over a constructor set; entries sort by
key then constructor id (settled), so one constructor's entries are
spread through the tree and cannot be rebuilt alone.

## ops/reindex/online

Postgres `REINDEX CONCURRENTLY`: build, catch up with writes made
meanwhile, swap. Settled online schema states as a staged job
(`engine/ddl-online-txn/staged-job`) give the staging.

```
reindex identities_by_user online
from j in sys::jobs where j.kind == Reindex select { phase: j.phase, done_pct: j.done_pct }
```

## ops/reindex/offline

Simpler; blocks writes to every constructor in the index's set.
