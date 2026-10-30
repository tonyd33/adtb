---
created-at: 2026-10-29
status: open
kind: feature
priority: 650
summary: Run the engine under Miri over simulated storage
---

# Feature: Run the engine under Miri over simulated storage

## Request

The adopted rulings project/arch/unsafe/miri-io/sim-storage and
project/testing/memory-model/miri ask for Miri over the simulated engine. The loom-miri track
(2026-10-29) runs Miri over the adtb-sys unit tests only. Run a slice of adtb-sim tests under
`work/tools/miri`, with storage simulated, and record the time it takes.
## Proposal

## Open questions
