---
created-at: 2026-10-27
status: open
kind: bug
priority: 857
summary: "Catalog::release removes the reader and its pending drops before rebuilding the enforced schema, so a failed rebuild leaves the stale enforced schema in place"
tags: [quality, engine]
---

# A failed release leaves the reader gone and the old enforced schema in force

## Report

`Catalog::release` at `crates/adtb/src/catalog/readers.rs:132-155` removes the bundle from `readers.live` and retains `readers.pending` first, then calls `without` and `checked`, either of which can fail with `Error::Internal`. On that error the reader is released and the pending drops are gone, but `readers.enforced` still holds the schema with those drops held. State is left half-updated.

Fix: compute the next `pending` and enforced source into locals, and assign `live`, `pending` and `enforced` only after both calls succeed.
