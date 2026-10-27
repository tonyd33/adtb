---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 179
summary: reindex r index (f) in S does not check that such an index is declared
closed: 2026-10-26
tags:
  - lang
---

# Bug: reindex accepts an index that does not exist

## Report

Index fields are stored as span-carrying AST with no span-insensitive comparison, so `reindex r index (f) in S` resolves `r` and type checks `f` but never matches it against a declared index.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in tlvmqsut. `reindex r index (f) in S` matches the checked fields, up to variable names, and the normalised scope against the declared indexes and uniques of `r`, and refuses an undeclared one. Commands keep their checked terms in `Adhoc.commands`, normalised into `NormalAdhoc.commands` and analysed by `adhoc_diags`. `explain B::q(args)` checks against a `Bundles` registry passed to `check_adhoc_recording`.
