---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 833
summary: "no error reason names a call on a closed database, so the engine reports Invalid"
closed: 2026-10-27
tags:
  - spec
---

# Feature: name a call on a closed database

## Report

`embedding.md` section 2.3 closes the database for every handle, but the error sums have no reason for a call made after close; the engine returns `Invalid` "the database is closed".

Ruling: add a `Closed` reason to the in-process error sum of `embedding.md` section 4.3, raised by any call, transaction or load on a database after `close`, with no effect.

## 2026-10-27: done

`RejectReason.Closed`, ctor id 469, is added to the prelude, `catalog.md` section 4 (sum, reason table, id table, unassigned range now from 470) and `embedding.md` section 4.3. Every call, transaction statement or commit and load on a closed database raises it. Test: `embedding::closing_one_handle_closes_the_database_for_its_clones` (load included).

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the q799 track.
