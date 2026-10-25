---
title: How waiting is made visible
pick: any
raised-by: [substrate/new-engine]
options:
  activity:
    title: A live sessions relation whose state is a closed sum of wait reasons
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
  sampled:
    title: Session states sampled at an interval into a bounded history
  cumulative:
    title: Time and count per wait reason, cumulative and per registered query
    links:
      - {rel: prior-art, to: prior/ops-mysql-pfs}
      - {rel: prior-art, to: prior/ops-query-store}
---
With readers that never block (SSI) the waits are I/O, latches,
log flush at commit, the safe-snapshot wait of declared read-only
transactions (settled `engine/retry/safe-snapshot`), the wait for a
concurrent inserter of the same key (settled `engine/wait-inserter`),
and admission.

```
type WaitReason
  = PageRead     { heap: CtorId }
  | LogFlush
  | Latch        { what: LatchKind }
  | SafeSnapshot
  | Inserter     { index: IndexId, xid: Xid }
  | Admission    { queue: Text }

type SessionState = Idle | Running { query: QueryRef } | Waiting { query: QueryRef, on: WaitReason }
```

## ops/waits/activity

pg_stat_activity `wait_event_type` and `wait_event`. Engine-defined
sums, subject to `ops/catalog-release`.

## ops/waits/sampled

Oracle Active Session History (unverified detail): a sample per second
per active session kept in a ring; answers "what was it waiting on at
3am".

## ops/waits/cumulative

performance_schema `events_waits_summary_*`; Query Store wait
categories per query and plan.
