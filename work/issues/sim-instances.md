---
created-at: 2026-10-30
status: resolved
kind: feature
priority: 300
summary: test crates compile about 1800 SimRuntime instances each; try rooting them once in adtb-sim as os-instances did for OsRuntime
closed: 2026-10-30
---

# Feature: Sim instances

## Request

os-instances took `OsRuntime` symbols in `update_concurrency` from 1818 to 0. Test crates still compile `SimRuntime` instances: `engine` 1835, `update_concurrency` about 1840, `calls` 814. Unmeasured: the saving of the same roots at `SimRuntime`, against the remaining 45 to 50 s per crate. Do not copy the list in crates/adtb/src/api/instances.rs: use a macro or a small root set. Measure rebuild after a one-line edit as well as a clean build.
## Proposal

## Open questions

## 2026-10-30

Landed: crates/adtb-sim/src/instances.rs roots 18 public functions at `SimRuntime`. Measured with nothing else building, 6 jobs: clean rustc CPU 1328.6 to 1039.1 s (-21.8%), rebuild after one line in pager.rs 90.6 to 73.9 CPU-s (-18.4%). `SimRuntime` symbols: update_concurrency 1857 to 16, engine 1845 to 3, calls 814 to 264. Roots in adtb-simkit left engine and update_concurrency unchanged. A full list saved no more. `apps/campaign` was not measured.

## 2026-10-30: resolved
