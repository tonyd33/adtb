---
created-at: 2026-10-26
status: open
kind: feature
priority: 775
summary: "three places drop temp relation rows by comparing relation ids against the declared count, an ordering stated nowhere"
tags:
  - quality
  - reference
  - test
---

# Feature: one helper drops the rows of temp relations after an ad hoc text

## Report

Three places run ad hoc items in order and then drop temp relation rows with `db.retain(|(r, _), _| (r.0 as usize) < declared)`: `crates/adtb-conformance/src/lib.rs:375-376`, `Fixture::text` in `crates/adtb-testkit/src/fixture.rs`, and `crates/adtb-testkit/tests/soundness.rs:241-243`. Each restates the assumption that temp relation ids follow the declared ones, and nothing states it where the ids are assigned.

Fix: one function in adtb-reference that drops the temp relations' rows, with that ordering stated once where temp relation ids are assigned.
