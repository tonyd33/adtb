---
title: Keeping live query results current costs work per commit
pick: any
raised-by: [net/feed/live]
options:
  rerun:
    title: Rerun a subscribed query when a commit overlaps its read set
    links:
      - {rel: prior-art, to: prior/net-convex-sync}
  incremental:
    title: Derive each subscription's result delta from the change set
    links:
      - {rel: prior-art, to: prior/constraint-ivm}
  admit:
    title: Admit as subscriptions only queries with a bounded incremental plan
    links:
      - {rel: same-as, to: constraint/admit/bounded}
  live:
    title: Live with subscription cost growing with subscribers times commits
    excludes: [net/feed/maintain/rerun, net/feed/maintain/incremental, net/feed/maintain/admit]
---
Constraints are already checked incrementally on change sets
(`constraint/enforce/delta`, `constraint/admit/bounded`); a
subscription is the same shape with a result instead of a refusal.

## net/feed/maintain/rerun

Convex. Read sets come from SSI predicate tracking, which the engine
already keeps.

## net/feed/maintain/incremental

DBSP-style delta rules over the change set.

## net/feed/maintain/admit

Registration refuses `subscribe` on a query with no index-bounded
delta plan, as constraints are refused.

## net/feed/maintain/live

Every subscription reruns on every commit.
