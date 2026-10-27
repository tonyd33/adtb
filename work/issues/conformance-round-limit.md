---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 781
summary: "the API cannot set a fixpoint round limit, so an unbounded fix runs 10000 rounds where the reference runs 200"
closed: 2026-10-26
---

# Feature: Conformance round limit

## Request

## Proposal

## Open questions

## Report

queries/11-fix-bounded takes about 50 s in debug on the engine, whose fixpoint limit is 10000 rounds; the reference uses 200. The API has no round limit per open or per call, so the engine backend of the conformance runner cannot match the reference. `.config/nextest.toml` gives the `adtb-conformance` `engine` binary a longer timeout until then.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as tylxvzor.
