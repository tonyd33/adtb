---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 744
summary: "an index or unique over an expression, such as a path into a sum field, is refused"
closed: 2026-10-26
---

# Feature: Expression indexes

## Request

## Proposal

## Open questions

## Report

Conformance files constraints/02-unique-on-sum-path and writes/06-upsert-expression-unique are refused at their schema: the engine does not build an index or unique over an expression.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as pyryqsnm. The engine failing list of the conformance suite is now empty.
