---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 99
summary: "check_adhoc silently skips commands, grants, revokes, roles, peers and temp declarations, so they pass unchecked"
branch: lang
tags:
  - lang
---

# Ad hoc commands, grants, roles, peers and temp declarations pass unchecked

## Report

`check_adhoc` in `crates/adtb-lang/src/check/mod.rs` checks only
`AdhocItem::Expr`, `Write` and `Transact`. Every other item matches
`_ => continue`: `Command`, `Grant`, `Revoke`, `Role`, `Peer` and
`Temp`. A wrong item of those kinds is accepted with no diagnostic, so
ad hoc text that should be refused checks clean.

Expected: each item kind is checked, or refused with a named reason
until it is.
