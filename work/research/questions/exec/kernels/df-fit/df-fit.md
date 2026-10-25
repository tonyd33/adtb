---
title: DataFusion's logical layer is SQL-shaped
pick: any
raised-by: [exec/kernels/datafusion]
options:
  physical-only:
    title: Use only DataFusion's physical plans, never its logical planner
  live:
    title: Live with lowering through DataFusion's logical plans
    excludes: [exec/kernels/df-fit/physical-only]
---
DataFusion's logical plans and type coercions follow SQL; NULL is
pervasive in its expression semantics (unverified for every operator).
The engine has no NULL and its own match lowering.

## exec/kernels/df-fit/physical-only

The engine's planner builds `ExecutionPlan` trees itself; expressions
are compiled from the engine's IR to `PhysicalExpr`.

## exec/kernels/df-fit/live

Lower to `LogicalPlan`, accept its optimizer rules.
