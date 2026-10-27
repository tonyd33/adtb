---
created-at: 2026-10-27
status: open
kind: feature
priority: 833
summary: "no error reason names a call on a closed database, so the engine reports Invalid"
tags:
  - spec
---

# Feature: name a call on a closed database

## Report

`embedding.md` section 2.3 closes the database for every handle, but the error sums have no reason for a call made after close; the engine returns `Invalid` "the database is closed".

Ruling: add a `Closed` reason to the in-process error sum of `embedding.md` section 4.3, raised by any call, transaction or load on a database after `close`, with no effect.
