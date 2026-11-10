---
created-at: 2026-11-10
status: open
kind: bug
priority: 300
summary: "the damage search classes a page Settled from the engine's own records, so a lost status page the engine wrongly records as settled and leaves unmarked passes"
tags:
  - quality
  - test
  - sim
  - class-weak-tests
---

# Bug: the damage search takes Settled from the engine's verdict

## Report

`judged` (crates/adtb-sim/tests/storage/damage_search/judge.rs:215) classes a page `Class::Settled` when `o.settled.contains(&w.pid)`, and `o.settled` (crates/adtb-sim/tests/storage/damage_search/consumers.rs:383) is the engine's `probe().records()` filtered to `Damage::Settled`. A settled page states `Nothing`, so an engine that records a lost commit-log status page as `Settled` when the retained log cannot prove it, and sets no mark, passes: the judge's expected value comes from the code under test. The row `(Settled, CommitLog, _)` is `Unasked::AtTier` (mod.rs:446), so it can be asked with no failure.

Fix direction: decide "settled" from the file with ledger.rs: a record of kind settled whose page's every xid has its outcome in the retained log; otherwise class the page `Damaged`.

Found in the quality review of `reviewed..main` at 7bf40103.
