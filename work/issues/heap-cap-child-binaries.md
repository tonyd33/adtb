---
created-at: 2026-10-31
status: open
kind: bug
priority: 487
summary: "A test's spawned child binary has its own heap cap outside the nextest slot arithmetic"
---

# Heap cap child binaries

## Report

A test that spawns a capped binary holds one nextest slot while the child has a further 1.5 GiB of its own, so slots times the default cap does not bound such a run. List the tests that spawn one (adtb-heap-cap's own runaway children, probe, shell, codegen usage tests) and give them the slots they cover, or a lower `ADTB_TEST_HEAP_CAP` for the child.
