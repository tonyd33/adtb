---
created-at: 2026-11-05
status: open
kind: feature
priority: 540
summary: "nothing in the spec says where an operator sees the log archive's state: archived through, last failure, lag"
---

# Feature: Operators cannot see the log archive's state

## Request

## Proposal

## Open questions

## 2026-11-05

From S6a. The archive's archived-through position and its last failure are reachable only through a doc-hidden Engine accessor. A failing destination silently grows the log directory (segments are kept until archived). Direction: a volatile sys:: relation (archived through, last success, last failure and its error, segments waiting), and a metric; a spec change to catalog.md through the design space.
