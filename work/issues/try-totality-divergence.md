---
created-at: 2026-10-25
status: open
kind: feature
priority: 702
summary: "typing says try e is total, which contradicts partial for an unbounded fix inside it"
tags: [spec]
---

# Feature: totality of try around a partial term

## Report

`analysis/totality.rs` gives `Kind::Try(x)` total whatever `x` is, following typing.md, which says `try e` is total. A `try` around an unbounded `fix` then counts as total although the fix may not terminate, contradicting "partial when it may not terminate". Rule through the design space: `try` absorbs failure, not divergence, so `try e` is as partial as `e` for termination.
