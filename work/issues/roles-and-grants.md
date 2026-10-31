---
created-at: 2026-10-27
status: in-progress
branch: roles-adhoc
kind: feature
priority: 410
summary: "registration and definer checks exist; call grants, binding and nonowner execution, system relation grants, ad hoc reads and writes, and management by text remain"
depends-on:
  - asserted-principal-value-domain
  - custom-principal-global-lookup
  - uuid-domain-textual-roundtrip
  - registered-argument-values
  - typed-permission-requirements
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

Fresh role-authority workspace ../adtb-role-authority on f305ffaf.
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

## 2026-10-28: paused for shared engine assertion validation

Fresh review found the existing shared Engine::principal accepts only outer
constructor/arity before cloning nested fields. The draft provider's textual
printer is also not a type validator. General engine ticket
asserted-principal-value-domain now takes full priority on main; no role-only
workaround is authorized. This repair lands first, then a fresh provider successor
rebases and reuses the shared boundary before completing native initialization.

Preserved role source efdd791fdb6d0d964fe340354b32f180403fe955 on e4d7aa6c,
including private test-only canonical full-counter fixture. Neither native
bootstrap correction nor principal validator was implemented. Two legacy reopen
fixtures timed out100 at70s with closed FileId worker panics, first reused runtime
and then fresh runtime. These are NOT successful capacity-refusal witnesses;
possible draft double-close after failed bootstrap remains source-inferred.
No leaked test groups remain. Tiny-memory guard passed on old bootstrap.

All evidence retained in source workspace and main target/role-authority-resume-
evidence:162copied nondebug files plus537archived tmp fixtures individually
SHA256verified before removing only completed debug cache. Original sources,
logs and fixtures remain. Handoff/receipt-role-authority-resume.md and blocked
review-role-authority.md describe the12-file unverified draft, exact three history
failures and pending native initialization/typed validation/review/full checks.
No role provider landing or server-readiness claim.

## 2026-10-28: shared principal engine repair landed

General engine validationbe84b407 is independently reviewed and full branch/main
verified (2,387root/47campaign/allstaticdocs). Ordered-type prerequisite928b7d38
and fixture-only repairfda78edb are independently reviewed/fullmainchecked too.
Before provider expansion, fix the separately observed custom root Principal
lookup bug under the existing root-first ruling. No-import registration must
work; an import workaround is not a user precondition.

Preserved provider efdd791f remains unlanded/unverified overall. Fresh successor
after engine prerequisites rebases, removes redundant printer-based assertion
validation in favor of shared Engine::principal, completes native empty-only
owner initialization, resolves retained legacy full-counter timeouts/double-close
hypothesis, then reruns private/public/spec/conformance/fresh review/full checks.
No producer/task/transport or nonowner execution claims follow from validation.

## 2026-10-28: root lookup verified, UUID formatter repair first

Shared root lookup landed1be643fa with full main2392root/47campaign and all static/
docs checks passed. Asserted-value and root-name prerequisites are resolved.
Independent UUID evidence confirmed that a valid short Uuid constructs and unwraps
but shared text/print fail Internal. User approved and lead adopted
spec/uuid-domain-format/lossless; fresh uuid-format track owns shared formatter/
parser and contract repair before provider resumption. Add that explicit dependency
so ready state reflects engine-first priority. Preserved efdd791f provider remains
unlanded and overall unverified, native owner initialization correction and
closed-FileId fixture diagnosis still unfinished. No new provider feature work.
After prerequisite, fresh successor must remove printer validation and rely on
shared Engine::principal, complete native bootstrap and all existing past guards,
then receive fresh exact-source review and full checks.

## 2026-10-28: engine prerequisites verified; native initialization next

The three explicit dependencies are now resolved. Root Principal lookup landed
1be643fa, approved total UUID formatting landed 1f403049, and the separate shared
newtype textual contract repair landed 2a44d5ab. The final combined main check
passed all 2,396 workspace tests and 47 campaign tests, plus lint, policy and docs;
full checked state and evidence are in textual-newtype-to-text-contract.

Provider source efdd791f on e4d7aa6c remains unlanded and overall unverified in
../adtb-role-authority. No provider change was made in these tracks.
Next fresh successor must rebase that preserved draft, remove printer validation
and consume shared Engine::principal, finish native empty-only owner initialization
before live statement assembly, and diagnose the legacy full-counter/closed-FileId
fixtures. Preserve the three existing past/history regression oracles and both
70-second timeout receipts; neither timeout established a capacity-refusal proof.
The native initialization plan and authority scope above remain unchanged. Fresh
exact-source review and branch/main full checks are still required before landing
any foundation slice. Broader permissions/binding/tasks/transport remain unfinished.

## 2026-10-28: fresh native initialization successor

User directed continuation. Fresh role-native workspace duplicates preserved
efdd791f onto verified main782701b9 without conflicts; initial change76787f55.
Old role-authority source and all failure receipts remain unchanged. One writer
owns the complete inherited12-file foundation and shared API; no other code
track runs. No dependencies added or broader permissions/transport scope opened.

Finish native owner initialization via existing Catalog::committed_rows/deploy
before live engine assembly and after open refusal gates; existing owner causes
no write, empty legacy state gets reserved private owner1 without counter draw,
nonempty ownerless state is malformed. Consume shared Engine::principal and
remove printer validation. Diagnose both retained closed-FileId timeout receipts;
do not treat them as capacity-refusal proof or mask an engine lifecycle defect.
Preserve unchanged past/history oracles and private canonical sequence fixture.

Acceptance: focused authority/private/public and past tests, strict affected
clippy/fmt/house/size, executable spec and conformance before fresh reading review,
full frozen branch check, independent architecture/exact-source review, lead
post-land full check. Record exact revisions, stable input/config manifests,
commands/status/logs/timing and unresolved failures. Local brief and predecessor
receipts are under work/briefs; durable scope is this ticket and its prior sections.

## 2026-10-28: authority foundation landed and fully verified

Landed `3cb67a82d3f4fa59d3478346bad320857db5b4a0`, `feat: add role authority
foundation`. Preserved `efdd791f` is now superseded evidence, not a draft to
resume. A fresh role-native author completed native initialization and shared
assertion validation. A fresh isolated reviewer cleared all 13 paths, source and
configuration manifests, architecture, tests and the adopted peer-removal spec
algorithm. No dependencies or rulings changed. This ticket returns to open: the
bounded foundation is complete; permissions and execution remain unfinished.

Canonical roles and deployment-capability grants persist independently of schema
hashes. Trusted embedded-owner administration issues opaque sessions scoped to a
database and immutable role identity. It checks live grants and assertion rights,
refuses stale/foreign/closed handles, and commits role/grant/credential/peer changes
with successful Grant audit through normal SSI transactions. Dropping a role
removes its peer mappings while preserving other roles' exact rows and ordinals.

Native initialization uses existing Catalog::committed_rows/deploy after open
refusal, migration and registration gates, before live engine construction.
An existing owner keeps its identity without a deployment write; empty ownerless
legacy state gets private owner 1 without a generated counter draw; nonempty
ownerless state is malformed. Assertions use shared Engine::principal, with no
printer validation. No authority cache, global administration mutex or pager
wrapper was added.

Bounded baseline `c0b02290` supplied the missing lifecycle cause: actual
full-counter capacity refusal, then Engine::shut closes the pager, then open
cleanup closes it again and the FileId 0 worker panics. The 8-second diagnostic
with 3-second kill grace ended with -9 and explicit child cleanup. It is failure
evidence, not a passing gate. Native placement removes the draft's late fallible
ownership path. Both original 70-second timeouts and the failed history oracles
remain preserved; those three history tests were not edited.

Full branch check passed frozen `e7e1a2fa9407a8bfcc9279c609c709c7f1c34166` on
`782701b9c652184c02a2767dfc6ac9042f35c7a2`, with all 2,567 tracked/configuration
inputs unchanged. Command from `../adtb-role-native`:

    direnv exec ../adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target python3 target/verify-native.py full

Collected exit 0 in 443.319588 seconds, from 2026-10-28T18:49:40.760557Z to
18:57:04.080145Z: 2,414 root tests passed, eight skipped (52.811 seconds);
47 campaign tests passed, one skipped (20.911 seconds). Lint, formatting, policy,
size, docs and final house checks passed. Focused checks first passed eight native,
ten public and 28 unchanged history tests plus strict affected checks. Nine spec
tests and all 234 conformance files passed BEFORE fresh reading review.
All 18 new default tests ran under two seconds. Full-counter legacy reopen keeps
every reservation byte and owner 1; existing owner 91 survives and later IDs
exceed 92. Two ownerless refusals followed by native reopen exercise cleanup.
Tiny-memory-zero remains a guard, not a claim that the old code had that defect.

Final reviewed `84e48259a5dcb19da1efcace49793beb09b4f9f6` adds only two prose
corrections to the full-checked source: deployment_id states its supplied-floor
contract, and the server contract omits internal owner-ID/native mechanics.
Separate formatting/house/size checks and fresh delta/hash review passed.
Rebasing to landed `3cb67a82` adds only ticket/index metadata; all 13 source paths
are identical to the final reviewed source.

Full main check passed clean `34ee57cc85226eb61cdec103dedd42a72c3e8deb`, parent
`3cb67a82`. Command from `../adtb`:

    direnv exec ../adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check

Collected exit 0 from 2026-10-28T19:00:02Z to 19:06:30Z (388 seconds, one-second
precision). All tracked inputs, local root/app Cargo configuration and revision
stayed unchanged. All 2,414 root tests passed, eight skipped (50.909 seconds);
47 campaign tests passed, one skipped (22.883 seconds). Lint, formatting, parity,
dependency, size and docs checks passed. Docs: ten tests in 10.144 seconds,
420 syntax inputs, 61 pages, 6,734 links, 4,999 fragments, no failures.
Permission, queue and execution-only timings are unknown. Session 25921 collected
exit 0. Log `target/role-native-integration/check.log` SHA256:
`ea45235459998c0bda8816c5a2d5c7d75236e4198ef7a6c6708b93b83f5109f3`.
Local receipts, review and handoff: `work/briefs/*role-native*.md`.

The completed branch debug cache was removed after a process audit and SHA-verified
backup of 796 nondebug files; original source, logs and fixtures remain. Earlier
completed fixtures were losslessly archived with individual SHA checks and restore
commands under `target/role-native-fixture-archives`. Unknown openperf and active
main cache were untouched. Pushing remains
deferred because the user reports Tangled down.

Next: relation/field and call/family permissions, registration/definer checks and
durable family-owner identity, then checked role binding and nonowner execution.
Syntax integration with statement atomicity, full audit/ledger and authentication
remain open. Engine-owned bounded resumable tasks/results follow authority and
binding; the codec alone is not a running server. No owner fallback is approved.
Preexisting native-open operating-error classification is not fixed here.

## 2026-10-28: next provider audit and admission prerequisite

Fresh authority-next-audit workspace starts2f080796 on39dc440e. Read-only source/
contract audit, no builds or implementation edits: recommend the smallest complete
next permission/binding provider, map canonical grant and family-owner identity,
and identify required kernel traversal/rechecking/atomicity boundaries. Existing
current rulings remain authoritative. Distinguish observable undecided behavior
from internal API choices; give concrete witnesses for any design gap. Lead owns
proposed decisions, never edits current without confirmation.

registered-admission separately repairs the shared argument-validation and refused
call-renewal boundary first. No authority/binding implementation or signature
consumer runs beside that writer. Audit acceptance: path/line ownership map,
bounded provider sequence, exact canonical state and checks, meaningful positive/
negative/concurrency tests, dependencies and any hard calls with recommended
resolution and reason. Preserve source; report in work/briefs/report-authority-next-audit.md
with a short successor handoff. Lead records essential findings here before
relying on ignored notes. No new dependencies or network authentication choices.

## 2026-10-28: next provider audit completed

Read-only audit of2f080796 on39dc440e finished unchanged, house0, no builds/tests.
No new independently reproduced engine bug. The next bounded implementation is
typed-permission-requirements, after the shared registered admission fix. This
broad ticket returns to open with no active branch. Fresh successor must start
from main, not the superseded role draft.

Provider order: complete pure typed requirements and effective coverage first;
canonical Log/Deployment grants with carry/dormancy/atomic rechecks second;
successful checked registration and durable family-owner identity next;
canonical call/family signature grants; then immutable checked binding and
nonowner execution. Management syntax later stages the text's changes atomically,
not one RoleAuthority commit per item. No owner fallback is an intermediate API.

Important follow-on boundaries: LogGrant holds stable target IDs but no grant-row
ID; vector positions are not identity. New Log rows must share the durable grants
allocator with Deployment rows, reconcile by typed provenance and preserve IDs
through applicable carry/rename/option changes without deleting Deployment rows.
Catalog::deploy's relation-wide no-migration-writer precondition needs an explicit
mixed-row ownership contract before both classes use catalog::grants. Native
apply rejects missing roles; recovery/restore folding is distinct, with dormant
grants explicitly canonical and never silently activated by recreated names.

Family ownership requires a successful-registration checkpoint independent of
unchecked headers and supporting zero-callable bundles. Header persistence still
precedes later failure; do not infer success/ownership from the earliest header,
an empty callable list or source display signatures. Binding uses immutable full
parameter AND result identity at its pin, checks exact call grants in the execution
transaction and on retries, and does not substitute current metadata for old pins.

Before the later family/call-grant providers, lead must settle two observable
witnesses through a proposed point after confirming the full surrounding rules:
an unversioned family grant when versions have different full signatures (audit
recommends require one unambiguous existing signature and freeze it); and legacy
header-only state that cannot distinguish failed checking from successful empty
registration (audit recommends explicit owner reconciliation before nonowner
ownership claims, never a guessed backfill). Neither is a prerequisite for pure
requirements work, no new refusal/reconciliation behavior is adopted now, and no
decision is being delegated back to the user without a concrete lead proposal.

## 2026-10-28: registered admission prerequisite verified

Shared engine fix744bddb0 is fresh reviewed and full branch/main checked:
2,426root tests,47campaign,allstatic/docs passed. All registered parameter
representations are checked and initially refused calls do not renew. Native
readiness hooks preserve one guard and the current retry/transaction owners;
they are not a future grant snapshot or per-retry authority check.

Next bounded provider is typed-permission-requirements, now ready. This broad
ticket remains open with that dependency. No role-backed execution, nonowner
registration/binding or management syntax was added in the admission repair.
Before a later public role ad hoc boundary, separately audit Prepared::new's
named host-parameter value/type assumptions; registered validation does not
certify that separate path. Engine defects confirmed there take priority under
the standing engine-first direction. Earlier family/legacy decision witnesses
remain follow-on work and do not block the internal pure provider.

## 2026-10-29: checked registration landed; family signature ruled

checked-registration (d6c535b5) is slice 3 with the live rechecks of slice 2
folded in; its ruling 2 settles the legacy header-only witness (such families
belong to the schema owner). The user adopted security/family-signature/unanimous:
a family call grant resolves the query in every live bundle of the family and
freezes the one signature they share; it is refused, naming the bundles, when none
declares the query or two declare different signatures. Next slice: canonical
call/bundle/family grants (audit item 3), then binding with BlockingMigration
notices.

## 2026-10-30: from grant-rev30

A signed family target compares by tokens, because comparing by ids at its own version (security.md 3.2, spec/signature-equality/ids) needs the family provider. `revoke call web::q at 41 (u: Key a::users) -> Text from x` with the type renamed since version 41 misses the row. Do this with family grants. Operator orphaning is only reachable through hand-written rows until operator grants have a non-owner grantor on a relation target; test it through the public path then.

## 2026-10-30: left by the grant rules of revision 30

With the family grant provider: `target_covers` has no family-covers-bundle case, so a revoke from a
role holding `call web::q at 41 ...` with option leaves a row it gave on `web-2.4::q` unrefused.
Latent, since no production path writes call rows yet.

`drop_role` leaves rows of the dropped role's grantees with no option above them. Ruling: refuse it
naming those rows, as a revoke that orphans them is refused.

## 2026-10-30

Ruling from server-fixes: a command result counts as the query that reads it. A role with `maintain` running `verify` while a finding holds a `witness` needs `read` on that relation. Until relation grants exist, it is refused with `Denied` and audited. Replace that with the real check here.

## 2026-10-30: audit and remaining slices

Read of main at 2e725780. A nonowner can take a session, register a bundle under grants it holds with option, retire or supersede it, run the commands its capabilities allow and give capabilities. It cannot run a query, a write or a registered call, including its own bundle's: all fail closed. Sections above that call definer checks or LogGrant identity unfinished are stale.

Rulings adopted: security/grant-absent/refuse, grant-bare-family/refuse, owner-calls/bundle-and-schema-owner, call-check-when/submit-and-start, family-grant-unread/refuse-when-callables, family-option-cover/equal-ids-now, drop-grantor/refuse, bind-needs-grant/some-call. security.md 2.1, 3.2, 4.1 and applications.md 5 state them.

Slices, each landable alone, none running nonowner work with owner rights:

1. Call grants: `RoleAuthority::grant`, the first writer of bundle, query and family rows; `signature::Ids`, printer and reader of `FAMILY::q at v ...`; `Entry.result`; signed revoke by ids; `target_covers` family covers bundle; `drop_role` refusal.
2. Checked binding and nonowner calls: `RoleAuthority::bind`, `RoleBinding::submit`, `owner` in server/task.rs deleted, `Denied`, `Call` and `Commit` audit rows. `BlockingMigration` waits for online migrations, split from this ticket.
3. Deployment relation grants and `read_files` in `EffectiveGrants`, and the revoke refusal of 3.5 for them. Absorbs lacking-repeats-per-bundle.
4. Ad hoc reads and writes for a role: remove the only-commands refusal in session.rs, `permission::adhoc`, real `explain`, `export`, `print schema`, `cancel call`, verify witness. Audit `Prepared::of_items` parameter values first. Absorbs permission-provider-follow-ups.
5. Management by text with statement atomicity; `schema` also needs `adhoc`.

## 2026-10-30: slice 1 landed

Call grants on bundles, queries and families are written by `RoleAuthority::grant`; `drop_role` refuses a grantor. Branch check exit 0, 3363 tests. Next: slice 2, binding and nonowner calls.

Left: `bundle_declares` compares declaration only, so `grant call web-2.0 to ann with grant option` is refused as a cycle by a family row at another signature. It fails closed and a revoke or the query form releases it. Compare ids when `calls.query` reads. `drop_role` takes the writer lock then enters, the reverse of `retire` and `supersede`; lock.rs spins so it does not deadlock, and 40 seeds finished.

## 2026-10-30: from slice 2

A `CallDescriptor`'s `Ty` names types by schema id, and a role's binder has no schema, so it cannot resolve them. The wire track builds the type descriptors of wire.md 7.2 from `RoleBinding`'s crate-private schema. Also there: wrong-length `args` gives `Invalid`, wire.md requires `Malformed`.

Slice 2 landed: `RoleAuthority::bind` and `RoleBinding::submit`, with no owner fallback. Branch check exit 0, 3394 tests. Next: slice 3, deployment relation grants and `read_files`.

## 2026-10-30: from slice 3, before a role can append migrations (slice 5)

Blocker for slice 5: a schema function reading `catalog::credentials` runs under owner rights, so `define fn creds () -> Set Bytes = from c in catalog::credentials select c.stored_key` with `grant call fn creds to web` lets `web` read verifiers through a direct call. Ruling: a schema function that reads `catalog::credentials` is refused at migration.

Also:
- A former schema holder can tell `revoke read customers` from `revoke read nosuch` (`gave`, api/server/mod.rs). Match its own rows before resolving.
- A confined read checks the path, then reads the resolved path: a symlink swapped between, by someone able to write inside a root, escapes. Open with no following, or read through the checked handle. A dangling symlink answers `Unreadable`, not `Denied`.
- `Bounds.confined` defaults to false, so a new server path fails open. Make it required.
- A bundle released by computed expiry, with no setting change, has no mark, so a wall clock stepped back brings it live. The revoke that relies on the expiry should write the latch; `latch` keeps only the highest renewal.
- `adtb-server` has no option to set `file_roots`, so it reads no files. `Disk::default()` with a relative glob finds nothing.

## 2026-10-30: from slice 3

A live file scan blocks migrations with `has no translation`. Not handled in slice 3.

Slice 3 landed: system relation grants and `read_files` count for a bundle owner, `file_roots` confines role sessions, expiry latches, and only the schema owner grants on `catalog::credentials`. Branch check exit 0, 3464 tests. Next: slice 4, ad hoc reads and writes for a role.

## 2026-10-31: slice 4 reviewed

Rulings: security/adhoc-names/charged, adhoc-function/call-and-reads, write-existence/stated, refusal-field/catalog-readers, system-read/monitor-or-read, export-occupied/refuse. Redaction stays per constraint for now and fails closed; 6.1 promises per row. Second round in the same workspace. A short text doubling through `let` costs exponential time and memory, also for the owner: bound it.
