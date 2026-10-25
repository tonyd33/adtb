---
created-at: 2026-10-25
status: open
kind: feature
priority: 850
summary: no tests check the schema operator laws or walk generated schemas
depends-on:
  - operator-log
  - engine-differential-tests
tags:
  - engine
  - test
---

# Feature: schema operator law tests and schema walks

## Report

Implement `docs/design/oracle.md` §5.7 and §5.8 over the generator of
§4: operator laws (inverse, data map, declared loss) over generated
databases, and the schema walk generator. Milestone 8 exits when the
laws hold over generated databases.

Prerequisites: `operator-log.md`, `engine-differential-tests.md`.
