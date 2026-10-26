---
created-at: 2026-10-25
status: open
kind: feature
priority: 733
summary: the catalog does not record the sequence counter page
---

# Feature: record the sequence counter page in the catalog

## Report

`write/sequence.rs` keeps the durable counters on one page, but the
catalog does not record its page id: the caller passes it through
`Setup.sequences`, as the simulator's `Roots` does. Record it in the
catalog state that survives crashes, so reopening finds it without the
caller.
