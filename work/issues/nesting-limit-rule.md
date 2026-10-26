---
created-at: 2026-10-26
status: open
kind: feature
priority: 191
summary: "the parser refuses text nesting over 64 levels, which the spec does not state, and counts each link of an or chain"
---

# Feature: Nesting limit rule

## Request

## Proposal

## Open questions

## Report

The front end track added `parse::MAX_DEPTH = 64` in `crates/adtb-lang/src/parse/mod.rs`, refusing "the text nests more than 64 levels deep". Every link of an operator chain counts, so a 65-term `or` chain is refused, which generated filters hit.

My ruling: the spec states a nesting limit of 64 levels, counting parentheses, operands, patterns, types, writes and generators. A chain of `and`, `or` or `union` builds a balanced tree, as a list literal does, and counts the height of that tree, not one level per link. Other operator chains count per link. Record it in the design space and state it in `docs/spec/syntax.md`.
