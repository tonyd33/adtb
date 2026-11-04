---
created-at: 2026-11-04
status: open
kind: feature
priority: 520
summary: A background vacuum that fails skips the relation for the process and shows nowhere
---

# Feature: Surface background vacuum failures

## Request

When a background vacuum fails with anything other than a cancel, the worker skips that relation for the rest of the process and the engine runs on. Nothing a user can see records it. A page error reaches quarantine on its own path; any other failure leaves dead versions growing with no sign. No existing surface fits: a `sys::verify_findings` `Structure` row claims damage and is replaced by the next scrub pass. The spec (catalog.md §11.5) does not say where such a failure shows.

Found in background vacuum V3, 2026-11-04.

## Proposal

## Open questions
