---
created-at: 2026-11-18
status: open
kind: feature
priority: 300
tags: [test, engine]
summary: no test opens a file written by an older release through a pinned bundle since the campaign app was removed
---

# Feature: Release upgrade coverage

## Request

The campaign app was removed on 2026-11-17 (maintainer: "not mature enough to support an actual app yet"). Its tests were the only ones to open a database file written by an older release under a newer one through a pinned bundle. Lost, from the tool-deletions review: engine.rs (bundle pins by hash for v1 and v4; releases 1.1, 1.2 and 1.3 opening over 1.0 and 1.1 files; 1.3 bundle calls over a 1.0 file; cross-campaign isolation; an SSI conflict between two writers), front_end.rs (schema checks, migration folds, every query checked at its pin), prep.rs (three migrations a foreign key refuses), live, live_retry and load (an older bundle's file upgrading, a double step, a collision workload). The stored-hash corpus kept the campaign's five migrations under crates/adtb/src/catalog/hash_corpus/.

## Proposal

A small fixture of releases (a schema, three or four migrations, a bundle per release) in a test crate: write a file at each release, open it under every later one through its pinned bundle, and check the rows and the bundle's calls. Generated releases would search more; the rule-cleanup plan's migration generator may subsume this.
