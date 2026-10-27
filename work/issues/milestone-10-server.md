---
created-at: 2026-10-26
status: open
kind: feature
priority: 1000
summary: "milestone 10 (the server) has not started: adtb-server is an empty crate, so no client can connect over a Unix socket or TCP"
depends-on:
  - milestone-8-embedded-alpha
  - server-engine-interface
tags:
  - milestone
---

# Feature: Milestone 10, the server

## Report

Tony: we want to connect over the network, by Unix socket and TCP, and serve many clients.

Milestone 10 of `docs/plan.md`. The design is settled: `docs/spec/wire.md` is the protocol, and the `net/` rulings in the design space cover transports, framing, handshake and bundle binding, multiplexed calls and transactions per connection, credit flow, in-band cancel, compression, row and Arrow results, admission with a bounded queue, authentication by password, certificate and Unix peer credentials, server-side retry of registered transactions, and idempotent writes. What is missing is the code: `crates/adtb-server` is one empty `lib.rs`.

Scope, from `docs/plan.md`:
- `adtb-server`: transports, framing, handshake and binding, calls, transactions, ad hoc text, appends, admission, feeds, pushes, idempotency, compression, protocol versions.
- Roles, authentication sources, grants, peer mappings, and audit with the ledger (security.md §2, §3, §5 to §8).
- Real-process isolation checks under kills and pauses.
- A client for the shell, so `adtb` can connect to a server as well as open a file.

Exit: the parity tests between the in-process API and the wire pass (wire.md §14, embedding.md §11).

Open before it can start: the two TODOs in `docs/design/server.md`, how the server submits a call to `adtb` and receives its batches, and how it binds a connection to a bundle. Tracked as `server-engine-interface`.
