---
created-at: 2026-10-26
status: open
kind: feature
priority: 783
summary: "the spec states no round limit option, no accessor for an in-process transaction's id, and no commit position for a read-only transaction"
---

# Feature: Spec api gaps

## Request

## Proposal

## Open questions

## Report

The API track added, in code only: `OpenOptions.fix_rounds` with `CallOptions.fix_rounds` and `TxnCallOptions.fix_rounds` overriding it, `AdHocTransaction::id()` returning the `TxnId`, and a read-only commit returning the position of the latest commit it read, else the log's durable end.

My ruling: all three are observable and undecided, so the spec states them as built. The round limit's default is 10000 rounds, and the error a fixpoint over the limit raises is the one the spec already gives for a bounded fix. State them in the API part of `docs/spec/`.
