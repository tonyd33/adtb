---
created-at: 2026-10-30
status: in-progress
branch: sim-instances
kind: feature
priority: 300
summary: test crates compile about 1800 SimRuntime instances each; try rooting them once in adtb-sim as os-instances did for OsRuntime
---

# Feature: Sim instances

## Request

os-instances took `OsRuntime` symbols in `update_concurrency` from 1818 to 0. Test crates still compile `SimRuntime` instances: `engine` 1835, `update_concurrency` about 1840, `calls` 814. Unmeasured: the saving of the same roots at `SimRuntime`, against the remaining 45 to 50 s per crate. Do not copy the list in crates/adtb/src/api/instances.rs: use a macro or a small root set. Measure rebuild after a one-line edit as well as a clean build.
## Proposal

## Open questions
