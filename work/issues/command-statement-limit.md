---
created-at: 2026-11-04
status: open
kind: bug
priority: 580
summary: The spec does not say whether StatementLimit bounds a command such as vacuum r
---

# StatementLimit on commands is unspecified

## Report

catalog.md §8.3 bounds "one query or write execution" with `StatementLimit`. No part says whether a command such as `vacuum r` honours it. The engine does not: a command's `HoldWait` carries no statement limit, so a paced `vacuum r` at budget 0 waits until cancel, a deadline or close.

Found in the background vacuum track (V2), 2026-11-04.

## Proposal

Decide in the spec whether a command's execution is bounded by `StatementLimit`, then make the engine and a test follow. A throttled command that ignores it can hold a scheduler worker indefinitely (see throttled-command-holds-worker).
