---
created-at: 2026-10-25
status: open
kind: feature
priority: 401
summary: "the interface between the pager and its clients (page kinds, page header offsets, Frame and PageRef, latching) is unstated, so heaps, trees and the commit log cannot start"
tags:
  - engine
  - design
---

# Feature: page and buffer-pool interface contract

## Report

`docs/design/storage.md` §1 and §2 and `docs/design/sys.md` §1 to §3
state the file, the page header and the page state word, but not the
interface a client of the pager programs against. Settle and write:

- `storage.md:86`: the offsets of `kind`, `owner` and `flags`;
- `storage.md:56`: the byte layout of a header copy;
- `sys.md:35`: what `Frame` and `PageRef` represent, and their methods
  (pin, optimistic read and validate, shared and exclusive latch,
  mark dirty);
- `runtime.md:140`: closing a file, file sizes and renames, as far as
  the pager needs them;
- how a client allocates and frees a page, and how a page change is
  logged (the record a client hands the log, `storage.md` §5.1).

The deliverable is the stated contract in `docs/design/` and the Rust
trait or types in `adtb`, with a test double clients can build against.
The B-link tree, the slotted heap and the commit log are built against
it while the pager is finished.
