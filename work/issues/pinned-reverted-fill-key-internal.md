---
created-at: 2026-11-14
status: open
kind: bug
priority: 290
summary: "a pinned bundle's insert translated through a reverted fill that prints a key fails as Internal: a value of a base form is a scalar, found Record"
tags:
  - engine
  - class-failure-flattened
---

# A pinned insert through a reverted fill printing a key fails as Internal

## Report

Migration 1 adds `O = { id: Key os, n: I64, x: Text }` with relation `os` and `Z = { id: Key zs }` with `zs`, and inserts into `os`; migration 2 drops `zs`, `Z` and `O.x`; a bundle `web-1.0 pins 2` whose txn does `insert os { n: 5 }`; migration 3 `revert 0002` with `fill O.x = match | O o -> print(o.id)`. On main the open succeeds and `call web-1.0` returns `Rejected { reason: Internal { what: "a value of a base form is a scalar, found Record([Int(13)])" } }`. Found in the review of the ahead-declared track (on main c10fb37c..ef08e67e); reproduced as an engine script, not kept.

The translated write evaluates the reverted fill's `print` of a key against a form that says scalar while the value is the key's record. Either the translation should carry the fill (the print of a key the pinned write supplies) or refuse it as Untranslatable with a reason; an Internal error is neither.

Direction: build the key's form from the same type the value is stored as (the key record), or refuse the translation naming the fill. Test: the history above as a bundles script expecting the call's exact outcome.

Defect class: Damage or failure gives a panic, hang or flattened error.
