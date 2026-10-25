---
title: "Which definition code a pinned bundle runs"
pick: one
options:
  own-version:
    title: "The definitions of its version; indexes, constraints and stored views use current code"
  current:
    title: "Current code everywhere"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/pinned-defs/own-version

The definitions of its version; indexes, constraints and stored views use current code. A pinned bundle's meaning does not change under it; an expression index is used only when code identity matches.

## spec/pinned-defs/current

Current code everywhere. Pinned queries change meaning silently.
