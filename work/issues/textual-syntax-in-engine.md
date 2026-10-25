---
created-at: 2026-10-25
status: open
kind: feature
priority: 745
summary: "the engine cannot read or print the textual syntax; it lives in adtb-reference over the reference's Value"
tags:
  - engine
  - lang
---

# Feature: The textual syntax for the engine

## Report

`adtb_reference::textual` reads and prints the textual syntax over the
reference interpreter's own `Value`. `adtb` does not depend on
`adtb-reference`, but dumps, error details and the REPL need the
syntax.

My ruling: move the reader and printer into `adtb-lang` over a
value-agnostic interface (a visitor or a small trait), with the
reference and the engine each implementing it for their values. The
round-trip property tests move with it.
