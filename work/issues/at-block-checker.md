---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 872
summary: "the checker types an at body at the current schema, takes no pins on an at block, and nothing raises WrongVersion"
closed: 2026-10-27
tags:
  - lang
---

# Feature: check at blocks at their version

## Report

`derived.md` section 6.3: an `at` block's body is typed at the schema of the position it names, `pins` fix that version, and a read whose relevant types differ from the pinned version's is refused with `WrongVersion`. The checker types every `at` body at the current schema and accepts no `pins`.

## 2026-10-27: done

The checker refuses an `at` body that is not a set and records `pins` in `Kind::At { pins: Option<u64> }`; registration refuses a callable whose `at` block pins a version after the database's. At run time the block is read at the version of its marker: a heap it reads that a migration made after the marker's version and after the version it is typed at is refused with `HistoryUnavailable`, and one made after only one of them with `WrongVersion { version }` naming the marker's version (`docs/design/mvcc.md` §10.3).

Open:

- The body is typed at the version the program is checked at, not at `pins` or the marker's version; what it reads must be stored alike at both, which the run-time check enforces only through heap identity: a relation or type renamed without a rewrite, or a function the block calls redefined, is not detected.
- The hash in `pins` is not compared with the log's.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the hcat track. What stays open is in `history-follow-ups`, `history-across-rewrites` and `heap-directory-interior-holes`.
