---
title: 32-bit ids force every version to be frozen before 2^31 transactions pass
pick: any
raised-by: [mvcc/xid-width/xid32]
options:
  freeze-per-sum:
    title: The batched per-sum vacuum also freezes, tracking an all-frozen bit per heap page
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  live:
    title: Live with a database that stops accepting writes near wraparound
    excludes: [mvcc/xid-width/wraparound/freeze-per-sum]
    assessment: {grade: broken, why: writes stop when an unvacuumed heap ages out}
---
Postgres: every table must be vacuumed at least once per 2 billion
transactions; warnings at 40 million from wraparound, refusal to assign
xids at 3 million (https://www.postgresql.org/docs/current/routine-vacuuming.html).
A constructor heap that is never updated still holds unfrozen xmins.

## mvcc/xid-width/wraparound/freeze-per-sum

Each heap keeps a `frozen_xid` (oldest unfrozen xmin); the per-sum
vacuum (`union-index/vacuum/batched`) freezes pages older than a
threshold and skips all-frozen pages. Freezing touches heaps only, never
the union index, so it adds no index pass. The sum's age is the minimum
over its heaps; an idle constructor heap is frozen once and skipped
after.

```
heap identities.Google  frozen_xid 1_200_000   vm: 2 bits per page (all-visible, all-frozen)
```

## mvcc/xid-width/wraparound/live

No freezing.
