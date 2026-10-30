---
created-at: 2026-10-30
status: in-progress
kind: feature
priority: 100
summary: most test time is spent in the engine; measure it and speed the engine up
branch: test-speed
---

# Feature: Test time engine

## Request

## Proposal

## Open questions

## 2026-10-30: scope

Tony: the next work is speeding up tests, engine speedups before the test framework or test
discipline, since an engine speedup speeds up all future work. Profile the suite, then fix the
largest engine costs. Correctness failures preempt tuning. engine-open-and-storage-perf and
campaign-engine-tests-slow are candidates, not assumed causes.
