---
created-at: 2026-10-25
status: open
kind: feature
priority: 560
summary: "vacuum requires that no write to the sum runs during it, and its orphan chunk sweep scans every version"
depends-on:
  - vacuum
  - ssi-integration
tags:
  - engine
---

# Feature: Vacuum alongside writers

## Report

`docs/design/mvcc.md` §10.1 makes it a precondition that no write to
the sum runs during a vacuum. The orphan out-of-line chunk sweep marks
every version of the sum on every vacuum, and also needs no value write
in progress. Expected:
- vacuum runs alongside writers, taking page latches as §2.4 states;
- the sweep is bounded by what changed since the last one;
- on-access pruning (`vacuum::prune`) is called by writers;
- progress shows as `docs/spec/catalog.md` §11.5 states.
