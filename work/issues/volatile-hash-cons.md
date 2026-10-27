---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: two identical calls of a volatile host function at different call sites of one query may be hash-consed into one evaluation
closed: 2026-10-27
tags:
  - lang
---

# Bug: identical volatile calls may share one evaluation

## Report

The normaliser now treats a call of a non-deterministic host function as a draw, so one call site is evaluated once per call. Two call sites with the same text in one query may still be hash-consed into one term and evaluated once instead of twice. Write a test with two such call sites and check each is drawn on its own.

## 2026-10-27: done

The normaliser's memo keyed a source term by its environment, so two calls of a user function whose body calls a volatile host function, with the same argument term, normalised to one shared term (`twice(t) = stamped(t) == stamped(t)` left one `stamp` call). The memo no longer records a result that draws. Unit tests in `analysis/tests.rs` count distinct host calls; `host_fns.rs` checks each call site draws on its own (the executor already evaluated the shared term per occurrence, so that test passed before the fix).

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qdrops track.
