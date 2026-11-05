---
created-at: 2026-11-04
status: open
kind: bug
priority: 520
summary: "Under explain analyze a write's plan sets actual only on the root node; catalog.md says every node has it"
---

# Explain analyze of a write omits actual below the root

## Report

## Report

catalog.md §7.4: under `explain analyze`, each node has a member `"actual"` with `"rows"`, `"loops"`, `"time_us"` and `"storage"`. For a write, the engine sets `actual` only on the root node. Not checked by any test.

Found in script batch 5, 2026-11-04; not reproduced by the lead.

## Proposal

Fill `actual` on every node of a write's plan, or find the contract that excuses the write's inner nodes. Add a test that walks every node of an analyzed write's plan.
