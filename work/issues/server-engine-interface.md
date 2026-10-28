---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 1001
summary: "docs/design/server.md leaves undecided how the server submits a call to adtb and receives its batches, and how it binds a connection to a bundle"
branch: server-interface-audit
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

## 2026-10-28: interface audit dispatched

User requested parallel server groundwork alongside campaign-live. Read-only
workspace server-interface-audit at /home/tony/code/adtb-server-interface-audit,
base dba817c5, owns assessment only; campaign-live owns apps/campaign.
Brief: work/briefs/brief-server-interface-audit.md. Report due at
work/briefs/report-server-interface-audit.md.

Verify historical TODOs against current public binding/lease, version/hash,
grants, registered calls, Arrow, cancellation, interactive transaction and
principal APIs. Classify stale documentation, missing contracts, missing
implementation and hard decisions. Recommend first end-to-end Unix-socket
milestone and separate codec, engine-boundary, transport/client tracks with
file ownership, landing order and parity tests. Retain TLS1.3+ALPN for TCP and
defined authentication for Unix. No code/spec/design edits or dependencies.
Lead settles hard decisions in proposed points for sign-off. No builds needed
for read-only audit; report path:line evidence and engine deficiencies promptly.
