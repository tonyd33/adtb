---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 1001
summary: "docs/design/server.md leaves undecided how the server submits a call to adtb and receives its batches, and how it binds a connection to a bundle"
closed: 2026-10-28
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

## 2026-10-28: audit complete, implementation deferred

Read-only audit complete; report work/briefs/report-server-interface-audit.md.
Binding/renewal already exist, but atomic version/hash/grant/metadata bind,
authenticated role authority, owned resumable tasks/results, bounded admission
and credit-held snapshots remain absent. Known tickets roles-and-grants,
embedded-open-follow-ups and sim-network-and-worker-loop cover major pieces.
No new campaign engine defect was established by this audit itself.

Recommend preserving owner-only embedded handles and adding an engine-owned
role/binding boundary plus runtime-independent resumable task/result handles.
Hard architecture choice needs a proposed point and user confirmation; none
created/adopted yet. First real server milestone should be authenticated Unix
bound calls with cancellation and genuine backpressure; TCP requires TLS1.3
and ALPN. Independent wire framing began under server-wire-framing, then
paused for confirmed concurrent-update-lost-advancement. No server capability
implementation before that correctness fix and boundary review.

## 2026-10-28: concrete boundary proposal awaiting confirmation

Proposed point server-owned-tasks extends current and accepts
net/task-boundary/owned plus net/task-resumption-cost/implement. Lead recommends
an engine-owned runtime-independent task/result handle, with resumable waits
and bounded production implemented before claiming server credit support.
Alternative runtime-specific futures and server-owned blocking adapters are
recorded, along with the machinery-cost objection. Current is unchanged; user
confirmation required before provider implementation. Details and code evidence:
work/research/questions/net/task-boundary/task-boundary.md,
work/research/questions/net/task-resumption-cost/task-resumption-cost.md and
work/research/notes/server-task-boundary-evidence.md. No runtime dependencies
approved by this proposal. Exact API signatures need one provider owner/review.
Campaign-live and pure wire framing are independent and resumed after verified
engine repair ef7361f9. Scope for first actual server remains authenticated Unix
bound calls with genuine cancellation/credit, zero optional client request ids;
full parity, TCP/TLS, translation and hosting are separate follow-on work.

## 2026-10-28: owned-task boundary approved

User approved server-owned-tasks; adopted into current with quandary adopt
--confirmed. Accepted net/task-boundary/owned and
net/task-resumption-cost/implement. Canonical contract work can proceed: one
provider owner specifies role/binding authority, task/result ownership, bounded
admission and suspension, cancellation/commit and interactive wait liveness.
Exact Rust signatures need review before implementation consumers branch.
Keep embedded owner calls compatible. No runtime dependency or transport
implementation is implied by approval.

## 2026-10-28: provider contract track prepared

Workspace server-boundary at ../adtb-server-boundary starts9cfcae7f; fresh
agent owns only docs/design/server.md. Concrete role/session/bind/task/result
contracts and cancel/commit/backpressure/interactive wait states come first,
with reviewed illustrative Rust ownership and a provider-first implementation
split. No executable code signatures, dependencies, transports or spec changes
in this bounded track. Acceptance: map authority/resource ownership for every
state transition, immutable pinned metadata, runtime-independent notifications,
one retry owner and real suspension/wait liveness; give queue/cancel/grant/
credit/commit tests for implementation. House/docs link checks for prose only;
full Rust suite intentionally not required. Lead reviews before landing and
provider/consumer implementation branches. Current separate campaign/wire
tracks touch disjoint files. Any engine bug restores engine-first priority.

Fresh dispatch attempted twice but rejected by harness agent-thread limit.
Workspace/probe/brief are prepared; implementation has not begun. Lead will
dispatch when capacity becomes available, preserving the fresh-agent rule.
This is a harness-capacity wait, not a permission or implementation failure.

## 2026-10-28: provider contract agent active

Fresh dispatch succeeded after wire worker completed. Agent owns prepared
server-boundary workspace/docs-only scope; access reverified. No executable
provider implementation yet. Campaign branch is independently checking its
final live slice; codec landed b27a1eb1 with full branch/main checks passed.

## 2026-10-28: resolved

Landed4fc009a2, docs/design/server.md sections1–3 specify the approved
engine-owned authority/binding/task/result and interactive registry contracts.
Lead exactf1555a31 review resolved drain-busy and unbound-ad-hoc consistency;
branch and scoped main docs checks passed0,7fixtures/420syntax/61pages/
6730links/4995fragments. Full combined main check after campaign also passed0,
2349root/46campaign/allstaticdocs. No provider implementation or runtime
behavior is certified by this design-only resolution. roles-and-grants remains
open for authority; server-owned-call-tasks tracks resumable implementation.
Exact Rust API signatures remain provider review work. First actual server
milestone remains authenticated Unix bound calls with real cancellation and
backpressure. No sockets or TLS stack landed.
