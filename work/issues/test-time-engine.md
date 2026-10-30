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

## 2026-10-30: verify's sweeps

Online verify added sweeps to the default run: about 15 s for the vacuum phase, about 10 s each for two concurrent-writer tests and the expression index test, about 3 s each for crash and repeated runs. Count these in the profile.
