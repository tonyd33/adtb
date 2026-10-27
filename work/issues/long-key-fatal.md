---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: a text key longer than about 1600 bytes fails with fatal TooLong instead of a rejection
closed: 2026-10-27
tags:
  - engine
---

# Bug: an oversized key is a fatal error

## Report

Inserting a row whose text key is about 20000 bytes fails with `fatal: storage failure: TooLong { len: 20013, max: 1618 }`. An oversized index key is the user's input, not a storage failure: reject the statement with a reason naming the key and its limit, and state the limit in the spec.

## 2026-10-27: resolved

Resolved in `06027c41` following the confirmed `spec/key-too-large/rejected`
ruling. Preflight every new row's index payload after row-size checks and
before constraints or physical writes. Return typed `KeyTooLarge` with
index identity and payload size/limit; preserve transaction usability.
Migration builds return `SchemaRefused`; structural failures remain fatal.

Independent correctness, spec and taste reviews passed. Integrated checks:
2327 workspace tests passed, 8 skipped; 27 campaign tests passed; clippy,
formatting, dependency policy and source-size checks passed. The combined
command reached its tool timeout after dependency checking; the remaining
source-size check passed separately. Corpus 9/9 and conformance 233 files
passed in the implementation workspace.
