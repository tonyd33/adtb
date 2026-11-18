---
created-at: 2026-11-18
status: open
kind: feature
priority: 250
tags: [tooling, test]
summary: "the snapshot runner's --update cannot rewrite refusal reasons or plans"
---

# Feature: Runner update errors

## Request

The snapshot runner in adtb-conformance (track snapshot-runner) writes back only result rows under `--update`; a case whose refusal reason or `plan` differs still fails and must be edited by hand. TQL's runner updates error sections only when asked (`--update=error`). Add the same opt-in for refusal reasons and plans, all or nothing per case as rows are now, never on the engine side for the conformance corpus.
