---
created-at: 2026-10-31
status: in-progress
kind: feature
priority: 400
summary: checker cases are compiled Rust wrappers instead of runtime-loaded input and expectation data
branch: checker-fixtures
---

# Feature: Checker runtime fixtures

## Request

## Proposal

## Open questions


## 2026-10-31: bounded B108 migration

Move the 108 mapped checker acceptance/refusal cases from Rust wrappers into
runtime data. Retain one shared driver and the checked/normalized binding-site,
phase, inferred-type and external-parameter assertions each case needs. The
three identified structural/resource exceptions and adjacent tests remain.
No production compiler, dependency or normative spec change is authorized.

Require complete case/subcase membership, execution of every discovered input,
independently reviewed exact expectations, no Rust rebuild on data-only edits
or additions, measured edit feedback/runtime tradeoffs, named mutation
sensitivity and exact restoration. Targeted checks precede independent review
and one final full check. This is parallel source work while the smaller
reference-fixture pilot finishes; builds wait for that check's resource slot.
