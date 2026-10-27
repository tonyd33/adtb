---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 857
summary: "Catalog::release removes the reader and its pending drops before rebuilding the enforced schema, so a failed rebuild leaves the stale enforced schema in place"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A failed release leaves the reader gone and the old enforced schema in force

## Report

`Catalog::release` at `crates/adtb/src/catalog/readers.rs:132-155` removes the bundle from `readers.live` and retains `readers.pending` first, then calls `without` and `checked`, either of which can fail with `Error::Internal`. On that error the reader is released and the pending drops are gone, but `readers.enforced` still holds the schema with those drops held. State is left half-updated.

Fix: compute the next `pending` and enforced source into locals, and assign `live`, `pending` and `enforced` only after both calls succeed.

## 2026-10-27: done

`Catalog::release` computes the next pending drops and the enforced schema into locals and assigns `live`, `pending` and `enforced` only after both succeed; a release that drops nothing keeps the enforced schema without rebuilding it. No test: the failure needs the checker to refuse a schema with only constraints dropped, which no history provokes.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
