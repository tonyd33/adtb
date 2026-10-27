---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 178
summary: "explain and other commands produce no checked term, so their targets are never normalised or analysed"
closed: 2026-10-26
tags:
  - lang
---

# Feature: Checked terms for commands

## Report

Found while fixing adhoc-skips-grants: `check_adhoc` checks commands (`explain`, `export`, `restore`, `dump`, ...) but keeps no term for them, so a consumer gets nothing to plan or run and no analysis covers their expressions. `explain B::q(args)` is not checked at all, since the bundle is unknown to ad hoc checking.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in tlvmqsut. `reindex r index (f) in S` matches the checked fields, up to variable names, and the normalised scope against the declared indexes and uniques of `r`, and refuses an undeclared one. Commands keep their checked terms in `Adhoc.commands`, normalised into `NormalAdhoc.commands` and analysed by `adhoc_diags`. `explain B::q(args)` checks against a `Bundles` registry passed to `check_adhoc_recording`.
