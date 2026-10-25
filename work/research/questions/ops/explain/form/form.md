---
title: The form an explained plan takes
pick: one
raised-by: [ops/explain/plan, ops/explain/analyze, ops/explain/stored, ops/explain/auto]
options:
  sum:
    title: A plan is a value of an engine-defined sum, queryable like any value
    needs: [engine/catalog-relations]
  text:
    title: Indented text for people, nothing for tools
  document:
    title: Text for people and a versioned self-describing document for tools
---
Tools compare plans across replans and releases. A plan that is data in
the engine can be queried; a plan that is text can only be read.

## ops/explain/form/sum

```
type PlanNode
  = Scan    { heap: CtorId, est: F64, actual: Option Actuals }
  | Probe   { index: IndexId, bounds: Text, est: F64, actual: Option Actuals }
  | Route   { arms: [{ ctor: CtorId, child: NodeId }] }
  | Pruned  { ctor: CtorId, at: PruneStage }
  | Join    { kind: JoinKind, build: NodeId, probe: NodeId, est: F64 }
  | ...

from n in sys::plan_nodes where n.plan == p
select match n | Pruned x -> x.ctor | Scan s -> s.heap | Probe _ -> ... | ...
```

The engine's own sum: a release adding an operator adds a constructor
and breaks every exhaustive match a tool wrote over it
(`ops/catalog-release`).

## ops/explain/form/text

SQLite `EXPLAIN QUERY PLAN`. Cheap, unstable, unparsed.

## ops/explain/form/document

Postgres `FORMAT JSON` beside text. Tools parse a document whose shape
the engine version names; new node kinds are new tags tools may ignore.
