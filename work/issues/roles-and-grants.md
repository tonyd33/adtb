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

## 2026-10-28: paused for general engine reservation prerequisite

Provider edits preserved in role-authority. Audit found Sequences draw/advance
publish in-memory reservations before persist can refuse counter-page capacity;
role identity must not rely on unconfirmed durable reservations. General engine
repair is isolated in sequence-repair; see sequence-reservation-after-refusal.
No role provider landed; no app workaround. Role worker records baseline private
unit witnesses and candidate evidence, then handoff. Fresh provider successor
must rebase onto verified repair and finish foundation acceptance/full checks.

## 2026-10-28: resume foundation after verified sequence repair

General engine fix96744632 is reviewed and full branch/main checked. Fresh
successor resumes role-authority draft6de7df13; rebase onto currentmain, preserving
baseline witnesses/patch/logs as evidence. Resolve overlapping sequence test
include/addition to the landed general implementation/tests, never reintroduce
baseline or old candidate. Complete ONLY settled canonical role/capability
foundation, no public role execution. Initial draft provider tests have unused
imports and a stdMutex spanning runtime call; fix before execution. Write real
provider contract and implemented-vs-planned status. Read prior handoff and
receipt-role-authority-paused.md. Full persistence/atomic/crash/concurrency tests,
strict lint/fullcheck and independent review/main integration still required.
Campaign-load-finish consumes unchanged embedded API; land campaign first, then
rebase provider onto it before provider fullcheck. One sharedAPI owner remains
role-authority; no transport consumer yet.

## 2026-10-28: approved peer mapping lifecycle

User explicitly approved proposedpoint role-drop-peers; adopted into current
through quandary. Ruling security/role-drop-peers/remove: atomically delete
peer_mappings rows naming a dropped role, preserve other mappings and ordinals,
change none on refused drop. Recreating the name requires explicit new mapping;
old external credentials must not inherit replacement identity. Existing catalog
representation unchanged. Canonical-row regression must demonstrate old retention
before applying correction; no authentication transport expansion.

Role-authority successor owns the one normative clarification in security.md2.1
reflecting this approved observable rule, plus design/server provider contract.
Read spec-revision skill, no additional normative choices: existing adopted
security/role-drop-peers question is the authority, do not duplicate or rename it.
Before fresh reading review run spec_corpus and conf conformance and retain exact
receipts, then reviewer writes algorithm/undecided inputs for changed paragraph.
Implementation/branchmainfull acceptance and all deferred surfaces unchanged.

## 2026-10-28: resume bounded provider after catalog guard

Fresh successor in role-authority resumes81c2bbc2 and rebases onto main15004a7f,
which contains full-verified general system-write guardd2716fb0. Preserve draft;
replace forbidden raw-catalog fixtures with private trusted setup/native calls,
complete approved atomic role-drop peer cleanup and security2.1 clarification,
and actual design provider contract. Existing broad and bounded scope remain
unchanged; no role execution exposed. Campaign lands first; provider may audit/
edit now but heavy builds wait for lead release, then rebase before verification.
Private fixture failure must demonstrate retained mappings before correction.
Spec_corpus and conf conformance precede fresh algorithm/undecided-input reading
review. Exact source architecture/correctness review, scoped checks and full
branch/main checks required; receipt must state all unfinished surfaces.
Do not change adopted header-before-migration ordering based on the closed false
bundle rollback oracle. Brief-role-authority-resume.md and prior handoff/receipts.

## 2026-10-28: family ownership remains a later provider

Foundation audit confirms api/bundle::record currently assigns every canonical
bundle row to permanent OWNER_ROLE, and no nonowner registration surface exists.
Applications3.1 defines family ownership by first registrant, which cannot be
inferred from lowest header ID: unchecked headers precede successful registration.
Do not invent that inference or change adopted header ordering. The bounded role
provider retains conservative canonical owner-row refusal, plus permanent-owner
and log-grant refusal. Nonowner bundle/family ownership, retired-bundle-sensitive
drop behavior and durable family-owner identity remain deferred with their
registration provider; no complete family/drop policy is claimed by foundation.
Any later exposure must implement those rights before accepting nonowner calls.

## 2026-10-28: provider review and native initialization correction

Campaign workload e4d7aa6c passed final branch/main checks and landed first.
Authority source30d1cb7d/basee4d7aa6c passed seven stable-input focused commands: 
5private native tests,8public simulator tests, strict clippy/fmt,9spec examples,
233conformance files on reference+engine,house;109.910s. Replaced custom row
ordering with existing header-aware catalog layout, removed redundant global
authority lock, separated requested audit flag from retained grant option, and
fixed test projection to retain audit row identity. Setup/compile/oracle failures
are preserved in target/authority-resume-* and receipt-role-authority-resume.md.
Actual peer-retention baseline5d79a7ee failed100: web ordinals1/7 survived drop
while other ordinal3 stayed unchanged. Matching deletes are now staged atomically
with role/grant/credential/audit changes. Native forced-overlap test proves an
internal SSI retry followed by role-absent Denied, with no orphan grant/audit.

Fresh independent review of all11paths found one initialization blocker, no
other provider/spec blocker. Generic bootstrap reserves a user counter slot and
flattens an ordinary capacity refusal into Corrupt. Source-inferred full-counter
case needs an exact legacy reopen witness before correction. Tiny-memory0 open
guard on78c16ee2 passed0 on old bootstrap, so memory refusal is NOT reproduced.

First fullcheck30d1cb7d collected100/stable,306.686s:2376of2379engine tests passed,
8skipped;3existing history tests failed because store::run(write=true) bootstrap
commits even when owner exists and no rows change. Campaign/later stages did
not finish. Preserve failures; do not modify those history oracles.

Lead approved correction: initialize permanent owner through existing native
Catalog::committed_rows/deploy during Engine::on after refusal/migration gates
and before normal write-path assembly. Preserve existing owner identity and
return without deployment when already initialized. Reserve internal ownerID1
only for empty ownerless legacy roles; nonempty ownerless canonical state is
malformed. No user counter draw, query-budget override, new public error variant
or fabricated migration/registration refusal. Live administrative mutations stay
on canonical SSI transactions. Add native legacy/full-counter and tiny-memory
guards, preserving baseline distinctions; rerun affected/spec/full checks then
fresh exact delta review and main integration. Narrow cfg(test) private sequence
fixture uses canonical persist, never handwritten bytes or public bypass.

Broader native-open error taxonomy is preexisting and not claimed fixed by this
metadata placement correction. Full roles/grants/binding/server remain unfinished.
