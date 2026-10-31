---
created-at: 2026-10-31
status: resolved
kind: bug
priority: 487
summary: "A test's spawned child binary has its own heap cap outside the nextest slot arithmetic"
closed: 2026-10-31
---

# Heap cap child binaries

## Report

A test that spawns a capped binary holds one nextest slot while the child has a further 1.5 GiB of its own, so slots times the default cap does not bound such a run. List the tests that spawn one (adtb-heap-cap's own runaway children, probe, shell, codegen usage tests) and give them the slots they cover, or a lower `ADTB_TEST_HEAP_CAP` for the child.

## 2026-10-31: fixed

Every test child goes through `adtb_heap_cap::test_child`, which caps it from the test's slots and stated concurrency. Clippy refuses a bare `Command::new`. Whole-file thread expects in two adtb-sim tests are noted on test-audit-findings.

## 2026-10-31: resolved
