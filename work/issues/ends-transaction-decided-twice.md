---
created-at: 2026-11-10
status: resolved
closed: 2026-11-16
kind: bug
priority: 550
summary: "whether a failed write ends its transaction is decided twice, by the write path from the unmapped error and by the API from the mapped one"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Bug: ends_transaction decided from two different errors

## Report

The write path calls `self.take_back(tx, &ran, error.ends_transaction())` on the error before mapping (crates/adtb/src/write/mod.rs:948, :984). The API then reports `ends` from `self.db.fail(f.error).error().ends_transaction()`, after mapping (crates/adtb/src/api/statements.rs:113-114). They agree today because mapping keeps the `By` class. A mapping that changes it would roll a transaction back while telling the client it stays open, or the reverse.

Fix direction: decide once: map before `take_back`, or carry the take-back decision in `Failed` and report that.

Found in the quality review of `reviewed..main` at 7bf40103.

## 2026-11-16, resolved

Gone since ec4ff7ad: `Failure` and `Refusal` carry one `ends`, decided from the unmapped error's class, and both `take_back` and the reply read it. Found stale by the audit-candidates triage.
