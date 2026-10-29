---
created-at: 2026-10-27
status: open
kind: feature
priority: 835
summary: "limits, feeds and bundle aggregates refuse registration; earlier test runtime gaps remain to be remeasured"
depends-on:
  - registered-argument-values
tags:
  - engine
---

# Feature: registered call follow-ups

## Report

- `Registered::call` checks the number of arguments but not their values against the parameter types.
- A declared `limit`, a feed, or a bundle aggregate refuses registration.
- Two tests in `crates/adtb-sim/tests/registered.rs` take 3.9 s and 2.2 s, over the 2 second budget, and the binary adds about 4 s of wall time to the default run.

## 2026-10-28: shared argument boundary repaired

registered-argument-values resolved in744bddb0. Shared representation validation
covers every declared argument in standalone, Arrow and interactive calls, with
Checked.schema and unused/imported parameter coverage. Full branch/main2426root/
47campaign plus strict/static/docs checks passed; receipts are tracked under
work/briefs/*registered-admission*.md. No spec or dependency change.

This broader ticket remains open for declared limits, feeds and bundle aggregates.
Earlier test-budget gaps require current measurements; this patch does not claim
to resolve them or turn suite timing into a comparative benchmark.
