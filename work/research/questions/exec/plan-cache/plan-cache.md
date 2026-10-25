---
title: When a registered query is planned
pick: one
raised-by: [evolution/breaks/invisible/registered]
options:
  at-register:
    title: Planned at registration, plan stored in the catalog
  first-call:
    title: Planned on first call and cached in memory per process
  custom-generic:
    title: Planned per call for the first calls, then a generic plan if not costlier
    links:
      - {rel: prior-art, to: prior/exec-pg-planner}
---
A registered query is already parsed and type-checked against a schema
version. A plan names constructor ids, which are never reused, so a
plan is valid exactly while the constructor sets it reads are
unchanged.

```
plan login_key#a1
  schema   12 (a91c...) identities {EmailPass #1, Google #2, Apple #4}
  stats    epoch 381
  params   (id: Uuid)
  tree     IndexScan identity_pk -> Route {1: .., 2: .., 4: ..}
```

## exec/plan-cache/at-register

Registration plans with current statistics and stores the plan (and
code, under `exec/model/latency/at-register`). Calls never plan.

## exec/plan-cache/first-call

Each process plans on the first call. Restart pays again.

## exec/plan-cache/custom-generic

Postgres prepared statements: five custom plans with the parameter
values, then a generic plan if its estimate is not much above their
average; `plan_cache_mode` overrides.
