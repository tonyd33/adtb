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

## 2026-10-30: profile

Engine 75% of suite CPU, simulator 11% of user cycles, test framework and testkit about 2.5%. Schema build and check are 44% of user cycles, `build_ahead` 30%. A prototype of a memo on `Source::built` and a faster `Vm::marked` took user CPU from 808 s to about 590 s with every test passing. Tracks, in order: schema-memo and catalog-insert now, then `Schema` behind an `Arc`, one application per migration, hardware CRC, and a shared simulator fixture sized by a new measurement. Cut no sweeps: nothing measured what a seed finds.
