---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 745
summary: "the engine cannot read or print the textual syntax; it lives in adtb-reference over the reference's Value"
closed: 2026-10-25
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

## 2026-10-25: resolved

## 2026-10-25: resolved

`adtb_lang::textual` holds the reader and printer over the `Textual` trait (contract in `docs/design/kernel.md` "Textual syntax"); the reference implements it for `Value`, the engine for `Val` in `exec/prim/syntax.rs`, and the engine's own 874-line copy is gone. `crates/adtb-sim/tests/textual.rs` checks that engine rows print as the reference prints the same query. Follow-ups are `textual-follow-ups`.
