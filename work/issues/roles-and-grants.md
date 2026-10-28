---
created-at: 2026-10-27
status: in-progress
kind: feature
priority: 830
summary: "no roles or grants exist, so the registration privilege check and every grant check have nothing to check"
branch: role-authority
tags:
  - engine
---

# Feature: roles and grants

## Report

`security.md` sections 2 and 3 define roles, grants and their checks, and section 4.2 requires a registering principal to hold the privileges its callables use. The engine has only the schema owner (`Service { name: "owner" }`), so registered calls run with every privilege and no grant is checked. In process this is invisible; the server needs it.

Fix: roles and grants in the catalog, the privilege check at registration, and grant checks on ad hoc text for a principal other than the owner. Then write the grant tests the regcalls track could not: a registration refused for a missing privilege, and a call by a principal without the grant.

## 2026-10-27: bundles

Also: registration privileges (applications.md check 5), retire and supersede privileges, and family owners are not enforced; every bundle is owned by "owner".

## 2026-10-28: provider audit and bounded implementation

Fresh role-authority workspace /home/tony/code/adtb-role-authority on f305ffaf.
First audit existing parsed role/grant forms, evolved operator grants and
catalog/deployment/embedded-owner boundaries. Lead will settle bounded provider
scope before code. One writer owns shared API exports; no transport consumers
until providers land. Campaign-load calls unchanged synchronous embedded API.
No dependency additions or network authentication codec selected.

Direction: persistent role/capability authority foundation, then complete
relation/field permissions and pinned bindings; no public execution that falls
back to owner privileges. Broader ticket stays open until all applicable grant
checks work. Acceptance includes role/grant persistence/failure atomicity,
deployment state excluded from schema hash, nonowner refusal and lifecycle/
revocation relevant to exported provider. Test exact scope in simulator,
clippy/fmt/house and authoritative branch/main checks. Durable final slice and
remaining gaps must be appended here before landing.

## 2026-10-28: authority foundation scope settled

Implement canonical catalog roles and deployment capability grants with trusted
embedded-owner provisioning and opaque database/immutable-role identity sessions.
No role-backed query or callable execution surface in this slice. Preserve the
existing synchronous embedded owner. Use real catalog::roles/catalog::grants,
durable atomic mutations and monotonic deployment IDs; no parallel authority
source of truth. Tests: bootstrap/reopen, schema-hash independence, denied nonowner
management, capability/assert checks, revocation/drop/recreate and stale or
cross-database handles, concurrent updates and failure atomicity. Changes may
include api/server, api/mod exports, narrow deployment ID reservation, simulator
tests and inter-subsystem contracts in docs/design/server.md. No new dependency.

Parser/checker currently discard role/grant forms as Checked::Nothing and
embedded preparation refuses them. Syntax integration, call/family grants,
relation-field permissions/definer registration checks, pinned binding and audit
are deferred provider slices; do not claim authentication/server readiness from
this foundation. Integrating syntax through existing separate deployment commits
would violate statement atomicity, so stage management transactionally in its own
later slice. All broader acceptance and unfinished gaps remain on this ticket.
