---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 760
summary: "the conformance suite does not run on the engine, and no differential, partition or plan-toggle test exists"
closed: 2026-10-26
depends-on:
  - in-process-api
  - conformance-runner
tags:
  - engine
  - test
---

# Feature: the suite and differential tests on the engine

## Report

Milestone 7 exits when the whole single-version suite passes on the
engine and the tests of `docs/design/oracle.md` §5.2 to §5.5
(differential, constructor partition, predicate partition, plan
toggles) pass over a fixed number of seeds per run. Add the engine as a
backend of the conformance runner, and those four tests against the
reference interpreter. Settle `oracle.md:79`, the generator interface
of §4.

Prerequisites: `in-process-api.md`, `conformance-runner.md`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as yloovzro: generator in `crates/adtb-testkit/src/cases.rs`, tests in `crates/adtb-testkit/tests/differential.rs`, contract in `docs/design/oracle.md` 4. No disagreement over 282 seeds; how strong that is goes to differential-strength.
