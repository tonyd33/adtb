---
created-at: 2026-10-25
status: open
kind: feature
priority: 405
summary: "a page allocated by one pager call and linked by the next leaks if a crash falls between them"
depends-on:
  - pager-and-log
tags:
  - engine
---

# Feature: Allocate inside an atomic change

## Report

The B-link tree, the commit log and soon the heap allocate a page, then
link it from another page in a later `update`. A crash between the two
calls leaves an allocated page that nothing refers to. Expected:
`docs/design/storage.md` §2.3 offers an `update` that allocates pages
and changes others in one batch, so the allocation and the link are
replayed together or not at all. `verify-structural.md` still reports
leaks, as a check.

## 2026-10-25: the heap too

The heap leaks a heap or directory page the same way: allocated and
owned by the heap, but not listed in its directory.
