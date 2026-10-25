---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 520
summary: "a crash between a B-link split and its parent update leaves a page with no downlink, and nothing repairs it"
branch: adtb-blink2
depends-on:
  - blink-tree
tags:
  - engine
---

# Bug: Incomplete B-link splits are never finished

## Report

A split is logged one level at a time. A crash after the split and
before the separator reaches the parent leaves the new page reachable
only by its left sibling's right link. Searches stay correct, but the
tree drifts from balance, and `verify` only tolerates it.

My ruling: mark a page whose downlink is not yet in its parent, as
Postgres's incomplete-split flag does. The next insert or `verify`
repair that reaches the page by a right link finishes the split. State
the flag in `docs/design/indexes.md` §8.1.
