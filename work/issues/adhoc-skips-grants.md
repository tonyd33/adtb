---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 99
summary: "check_adhoc silently skips commands, grants, revokes, roles, peers and temp declarations, so they pass unchecked"
closed: 2026-10-25
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

## 2026-10-25: resolved

Fixed in `fix(lang): decimal literals, constructor joins, aggregate sources, anonymous sum indices and ad hoc checks`. Commands, grants, revokes and role, peer and temp relation statements are checked and have no term.
