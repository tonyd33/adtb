---
title: What protects a registered query from a worse plan after replanning
pick: any
raised-by: [exec/replan/stats]
options:
  pin:
    title: An operator pins a plan; statistics drift no longer replans it
    links:
      - {rel: prior-art, to: prior/ops-query-store}
  baseline:
    title: A replan is recorded unaccepted and used only after it is verified no worse
    links:
      - {rel: prior-art, to: prior/ops-oracle-spm}
  last-good:
    title: The engine reverts to the previous plan when a replan regresses measured cost
    links:
      - {rel: prior-art, to: prior/ops-query-store}
  hints:
    title: Hints attached to a registered query in the catalog, outside its text
    links:
      - {rel: prior-art, to: prior/ops-query-store}
---
Registered queries are planned at registration and replanned on schema
change and statistics drift (settled). Replanning on drift is where a
plan regresses silently: a constructor heap grows from empty and the
new plan is worse for the parameters actually used.

A plan names constructor ids and index ids, never reused, so whether a
pinned plan is still valid after a schema change is decidable: every
heap and index it names exists and the constructor sets it reads are
unchanged. SQL Server's forced plan instead fails at recompile with
`NO_INDEX`.

```
pin plan sys::plans#8812 for queries::login_key
from p in sys::plans where p.query == queries::login_key
select { id: p.id, state: p.state, stats_epoch: p.stats_epoch, mean_ms: p.mean_ms }
```

## ops/plan-stability/pin

Query Store `sp_query_store_force_plan`. Kept across restarts and
stats replans; a schema change that invalidates it unpins it and
reports the reason.

## ops/plan-stability/baseline

Oracle SPM: new plans go to history unaccepted; evolution runs them,
accepts those at least as good. Needs a way to run a candidate plan:
under `exec/model/adaptive` a candidate can run interpreted on sampled
calls.

## ops/plan-stability/last-good

SQL Server automatic plan correction: force the last good plan when
the new one's CPU is measurably worse (over 10 s estimated gain), then
unforce it if forcing is not better.

## ops/plan-stability/hints

Query Store hints: hints keyed by query id rather than written in the
text. Registered query text is checked and versioned in a bundle;
hints written in it would make a plan change a new bundle.
