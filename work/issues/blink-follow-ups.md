---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 542
summary: "a parent's first child is never unlinked, dead-entry cleanup is not wired to heap visibility, and fence keys cost space"
closed: 2026-10-25
depends-on:
  - blink-space-reuse
tags:
  - engine
---

# Feature: B-link follow-ups

## Report

From page deletion and space reuse:
- a parent's first child, the rightmost leaf of a level and inner pages
  are never unlinked. The first child needs a way to latch its left
  neighbour under the parent's left sibling;
- `Index::insert_with` takes a dead-entry judge, but nothing in `mvcc`
  passes the heap visibility check to it yet;
- fence keys cost about a quarter more leaves on ascending inserts.
  Suffix-truncate separators and fence keys;
- `FORMAT` stayed 1 although the node layout changed. Bump it before
  any database is persisted, and add a decoder test for the old layout
  if one is kept.

## 2026-10-25: resolved

Landed as `feat(btree): unlink first children, truncate leaf separators and bump the node format`. Truncation made test trees shorter and some height minimums were lowered; `blink-test-heights.md` restores them.
