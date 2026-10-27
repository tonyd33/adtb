---
created-at: 2026-10-26
status: open
kind: feature
priority: 1001
summary: "docs/design/server.md leaves undecided how the server submits a call to adtb and receives its batches, and how it binds a connection to a bundle"
tags:
  - server
  - design
---

# Feature: The server's interface to the engine

## Report

`docs/design/server.md` has two TODOs that block `adtb-server`:
- how the server submits a call to `adtb` and receives its result batches, as a task on the engine's worker scheduler;
- how the server binds a connection to a bundle at its version through `adtb` (wire.md §3.4), renewing its lease.

The ruling `net/parity/mirror` says each wire message is an in-process call serialized, with the server as a library client, so the answer should be the in-process API of `embedding.md` plus whatever streaming and cancellation hooks the wire needs. Settle both in the design file, as postconditions the engine provides, before the server track starts.
