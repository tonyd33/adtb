---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 1002
summary: adtb-server has no checked wire primitives or incremental frame codec
closed: 2026-10-28
tags:
  - server
  - test
---

# Feature: Server wire framing

## Report

User requested parallel server work. The empty adtb-server crate has no
implementation of wire.md's primitive table or section 2 frame/chunk format.
These settled byte-level rules can be implemented before authenticated engine
sessions and resumable execution are available.

## Scope and verification

Workspace server-wire at ../adtb-server-wire, base62d77710.
Owner files crates/adtb-server/{Cargo.toml,src/lib.rs,src/wire/**,tests/wire_*.rs}
and necessary lock delta. Campaign-live separately owns apps/campaign; landing
order independent. Brief work/briefs/brief-server-wire.md; handoff and receipts
due at work/briefs/handoff-server-wire.md.

Implement canonical LE/LEB128/bytes/NFC text/options/lists/position/version
primitives, checked frame headers, incremental partial/coalesced frame decoding,
chunk splitting and bounded interleaved same-kind reassembly. Limit frame,
message and stream memory explicitly. No sockets/auth/TLS/connection lifecycle,
engine edits, heap codec reuse, full Value/Arrow codec or compression claims.
Only approved added dependency: existing allowlisted unicode-normalization
0.1.26 with default features disabled.

Acceptance: golden bytes, exact structured malformed-input errors, overflow/
noncanonical/UTF8/NFC/count/length boundaries, every-byte partial splits/EOF,
interleaved chunks, illegal kind changes and resource bounds. Focused server
tests/clippy/fmt, then from workspace root:

```sh
direnv exec ../adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check
work/tools/house-check
```

Record tested/base full ids, input stability, exact scope/command, collected
status, retained logs and incomplete stages. Lead owns review and post-land
combined full check. No full network-server milestone claimed by codec alone.

## 2026-10-28: codec resumed after engine repair

Fresh successor continues preserved server-wire, rebased onto verified main
eb2f61637b74; partial source ff0a106d remains untested. Scope and acceptance
above unchanged: pure primitives/framing only. Finish audit, structured length
mismatch, resource/compression contracts and golden/adversarial tests; affected
tests/clippy then stable full check. No sockets or engine-interface changes.
Engine-first rule still applies. Lead owns review, landing and main integration.

## 2026-10-28: resolved

Landed b27a1eb1 after lead exact eb53dbb1 review and branch check exit0:
2349root/8skipped,40campaign/1skipped, all static/docs. Main integration also
passed collected exit0 at clean WC e4d97725bfa1cc0aa986d91350b9c4c68c137648
(parent b27a1eb1),2026-10-28T03:21:23Z to03:22:41Z,78s elapsed. Revision and
tracked/local-config hashes match before/after. Exact main command: direnv exec
../adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check.
Root2349passed/8skip, campaign40passed/1skip, all static/docs stages passed.
Rust1.98.1/default profiles/features; permission/queue/execution-only unknown.
Logs/status/hashes in target/server-wire-integration/. Branch evidence copied
to main target/server-wire-evidence/; only completed sibling target/debug
removed14GB for campaign headroom. Source and original logs remain.

Codec includes15golden/adversarial tests, canonical primitives and ordered
option lists, bounded incremental framing/chunks/reassembly with typed errors.
Bounds measure logical accepted body bytes/items/partial streams, not total
process memory or caller-retained completed messages. No sockets, authentication,
compression execution, body Value/Arrow encoding or connection lifecycle/fair
scheduling is implemented. Those remain separate server/engine work.
