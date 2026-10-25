---
created-at: 2026-10-25
status: open
kind: feature
priority: 178
summary: "explain and other commands produce no checked term, so their targets are never normalised or analysed"
tags:
  - lang
---

# Feature: Checked terms for commands

## Report

Found while fixing adhoc-skips-grants: `check_adhoc` checks commands (`explain`, `export`, `restore`, `dump`, ...) but keeps no term for them, so a consumer gets nothing to plan or run and no analysis covers their expressions. `explain B::q(args)` is not checked at all, since the bundle is unknown to ad hoc checking.
