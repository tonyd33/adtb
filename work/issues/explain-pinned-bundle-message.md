---
created-at: 2026-10-29
status: open
kind: bug
priority: 750
summary: explain of a query in a bundle pinned earlier says the bundle is not registered
---

# explain of a query in a bundle pinned earlier says the bundle is not registered

## Report

Found by the manual-gaps track on 2026-10-29. `explain B::q(args)` for a bundle pinned at an
earlier version answers "bundle `B` is not registered", though the bundle is registered and
callable. For a current bundle it answers "`explain` does not run in this engine yet". The test
at crates/adtb-sim/tests/bundles.rs:2969 asserts the wrong text. Fix: answer the same not-yet
refusal for both.
