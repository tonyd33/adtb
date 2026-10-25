---
title: How a query plan is inspected
pick: some
options:
  plan:
    title: explain returns the chosen plan without running the query
    links:
      - {rel: prior-art, to: prior/ops-pg-explain}
  analyze:
    title: explain analyze runs the query and reports actual rows and time per node
    links:
      - {rel: prior-art, to: prior/ops-pg-explain}
  stored:
    title: The stored plan of a registered query is read from the catalog
    needs: [exec/plan-cache/at-register]
  auto:
    title: Plans of executions over a threshold are recorded with their actuals
    links:
      - {rel: prior-art, to: prior/ops-pg-explain}
---
A plan over a sum has shapes a SQL plan lacks: one arm per constructor
of a `match`, union-index probes whose entries are routed by constructor
id, and constructors pruned at compile time or from parameters at start
(`exec/prune`). Inspection has to show which arms ran and which were
pruned.

```
explain from i in identities where i.user == u
        select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub

Probe identities_by_user (user == u)
  Route by ctor
    #1 EmailPass  est 0.4   -> Project email
    #2 Google     est 0.5   -> Project gid
    #4 Apple      est 0.1   -> Project sub
```

## ops/explain/plan

Plan only, with estimates per arm from per-constructor statistics
(`exec/stats/per-ctor`). Postgres `EXPLAIN`.

## ops/explain/analyze

Runs the query and annotates each node with actual rows, loops, time and
pages read. A write is a change-set value (`write/surface/txdata`), so
analysing a write computes and checks the change set and discards it
instead of committing; no `BEGIN ... ROLLBACK` dance as in Postgres,
where the side effects of `EXPLAIN ANALYZE` happen.

```
explain analyze update g in identities.Google where g.domain == "corp"
  set Google { g with domain: "corp.example" }
-- change set: -1204 +1204 in identities.Google; constraints checked; not applied
```

## ops/explain/stored

At-registration planning stores the plan; inspecting it reads the
catalog, so what is shown is what calls run, not a fresh replan that
may differ.

```
from p in sys::plans where p.query == queries::login_key select p.tree
```

## ops/explain/auto

auto_explain: executions slower than a threshold record the plan with
actuals into a bounded log. Recording actuals costs per-node timing on
every run; Postgres warns `log_analyze` slows all statements.
