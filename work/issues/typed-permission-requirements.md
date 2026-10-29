---
created-at: 2026-10-28
status: in-progress
kind: feature
priority: 829
summary: no typed provider extracts and checks complete relation/field/write privilege requirements
branch: permission-successor
depends-on:
  - registered-argument-values
  - fix-row-provenance
tags:
  - lang
  - engine
  - test
---

# Feature: Typed permission requirements

## Report

No provider computes complete typed permission requirements for a checked query
or transaction. The engine's api/reads.rs records reader invalidation metadata;
it loses relation/constructor field provenance and collapses write operations,
so it cannot authorize execution. Source::grants resolves stable target IDs but
is not itself a complete effective authority snapshot. Nonowner registration
and execution must not rely on either as if authorization were complete.

## 2026-10-28: bounded provider recommended by audit

Read-only authority-next-audit checked empty2f080796 on39dc440e, no source edits,
tests or builds. Exact report/handoff are local work/briefs/*authority-next-audit*.
This ticket preserves essential scope independently of those ignored reports.
Implementation starts from current main only after registered-argument-values
lands and passes main verification. No worker is assigned yet.

Build one complete internal typed requirement/coverage provider in adtb-lang.
No role execution export, canonical grant store, authentication, binding, task
queue or implicit owner fallback. Input is the checked resolved language graph
and an explicit effective grant snapshot over the same identity domain; output
is deterministic complete missing requirements in ordinary-use or registration
mode. Registration requires grant option for relation requirements; file-read
capability is distinct and must not inherit that relation-only rule.

Track stable relation, constructor and field IDs for reads; insert/delete sets;
same-constructor changed-field updates; exact source-to-target retag edges; and
file-read requirements. Preserve origins through aliases, matches, nested values,
dereference, language functions, recursion and user aggregates. Ranging, exists
and count still require the selected constructors and their implicit key read.
An unrelated parameter of a row type is not a read of every relation of that type.

Retain checker-origin access/write-purpose facts through normalization, or finish
extraction before normalization erases them. Do not assume arbitrary normalized
Terms suffice. Checker spread/header/key reconstruction emits ordinary Field
nodes; copying unchanged secret fields is not a read/update privilege on them.
An explicit predicate, result or comparison using that secret is a real read.
Returning or transporting a whole source row must demand its full actual reads.
Document this inter-subsystem contract before consumers depend on it.

Replace preserves semantic update identity; follow path-specific constructor
cases rather than a Cartesian product of widened input/output types. An identity
arm needs no update or retag grant. Delete+insert remains distinct from retag;
upsert includes proposed insert plus conflict-branch old/new provenance; Rejects
includes insert into its destination. Automatic referential actions use schema
owner rights under existing rules and are not charged to the bundle owner.
Keep directly imported schema-function call policy distinct in later adapters.

Coverage consumes already-resolved, applied effective field sets. It must not
expand raw omitted-field Source grants against an arbitrary later schema.
Canonical application-time freezing, grant carry, dormancy, durable row identity
and transaction snapshot ownership belong to the subsequent engine provider.
No second mutable authority store or text-based target identity belongs here.

Ownership: new adtb-lang/src/permission module (name illustrative), lib.rs export,
narrow checker/normalizer metadata, affected language tests and design/kernel.md
contract. One fresh writer owns shared language signatures and callers. No new
dependencies or normative spec changes. Use existing kernel/write distinctions
and resolved IDs; report any observable gap with a concrete witness to lead.

Acceptance: positive and negative read coverage across shared row types/relations,
restricted constructors, implicit keys, reference following, whole-row reads,
nested output and field provenance; update public while carrying secret versus
explicit secret read; unchanged identity arms and exact retag edges; upsert
conflict paths, rejects, transitive functions/aggregates/recursion and file reads;
ordinary versus grant-option coverage and deterministic complete diagnostics.
No wildcard that silently omits a new kernel form. Preserve existing language
semantics and optimizer behavior. New default tests stay under two seconds unless
using configured sweeps. Strict affected lint/fmt/house/size, full frozen branch,
fresh independent architecture/contract review and full main verification.

## 2026-10-28: implementation dispatched

User directed continuation. Fresh permission-requirements workspace starts
060ff07d on999926a8 (all prior registered admission fixes verified). One writer
owns shared language signatures, new provider, narrow checker metadata, necessary
callers/tests and design/kernel.md contract. No other code/build track runs.

Before production edits the writer presents an algorithm/API/ownership map for
lead review. Direction: capture semantic access/carry/write purpose at checking
and compute requirements before optimizer normalization erases it, or carry an
equivalent small typed effect summary. No duplicate parser/AST, shadow language,
reader-metadata authorization or blanket field/constructor approximation. The
complete pure provider scope and acceptance above remain unchanged. No public
execution, canonical persistence, new dependency or normative change is approved.

Acceptance commands: targeted adtb-lang permission/checker tests, strict affected
clippy, cargo fmt --all --check, house/size checks, design example/docs checks as
applicable, one frozen full branch check, fresh independent exact-source and
architecture review, lead work/tools/land and full main check. Record source and
configuration hashes, commands/exits/logs/timing, failures and any unfinished
stage. Report new tests' default runtimes. Confirmed engine bugs preempt expansion.
Local brief work/briefs/brief-permission-requirements.md; durable scope lives here.

## 2026-10-28: approved purpose-preserving normalization boundary

Writer's first architecture proposal was reviewed before production edits.
Use the existing language normalizer while preserving checker access/carry purpose
as Terms metadata. Requirements wrappers accept checked Query/Txn/Adhoc, normalize
internally, then analyze only those purpose-preserving graphs. No public analyzer
over arbitrary execution Terms and no second higher-order/partial evaluator.
This follows queries12.4's normal-form constructor reads and existing erased-total-
argument behavior. Explicit projection that collapses onto a carry Field clones
an explicit residual node; it must not mutate a shared carry's purpose globally.
All relevant copy/rewrite/arena paths must preserve this contract.

Typed atoms cover constructor/field reads, insert/delete, update fields, exact
retag edges and ReadFiles. Applied effective read grants implicitly cover declared
key fields, not storage's physical whole-row identity. Same-site/binder IDs alone
do not prove row identity across repeated relation/function reads. Exact-copy
tokens preserve actual old-row copies, unrelated reads get distinct evidence,
and joins only retain equal exact-copy evidence. Field-recursive types are refused
by schema checking; references stay opaque, so type-shaped monotone joins have
finite height. No arbitrary iteration/depth cutoff or guessed identity is allowed.

Implicit upsert conflict/index/constraint lookup does not charge a user Read by
itself; explicit old fields/whole-row use charges their origins, and proposed new
values carry only actual source reads. Pair Insert-only conflict-skip with explicit
old-read negatives and returned old-value coverage. This is implementation of
existing privilege/constraint separation, not a new normative ruling.

User requested parallel work. Disjoint campaign-seat-cleanup owns only app player
helper, manual-derived owns three manual pages. No shared public API signature
they consume changes here. They land MANUAL -> CAMPAIGN -> PERMISSIONS; permission
writer rebases onto verified predecessors before final acceptance. Rust builds
are serialized: campaign app checks first, manual shell/oplog second, permissions
next on explicit lead release. Source editing proceeds concurrently. Lead owns
scoped manual/docs and full main checks; no other actor may write language files.

## 2026-10-28: paused for shared checker fixpoint provenance

Focused acceptance reproduced the fix-row-provenance witness against existing
checker logic; writes3.1 admits that key-preserving fix target. Root isolated a
fresh main-based prerequisite rather than deleting the test or adding a permission
exception. This track retains its full partial source and ends after current
source checks/receipt. No full branch pass or completed permission provider.

Third focused run:36/37passed, sole failure the unchanged update-over-fix checker
refusal. Earlier unsupported fixture forms (keyless schema, parenthesized direct
lambda, order tuple syntax) are retained separately from the actual provider
overread: synthetic group/tuple/record transport Fields initially consumed carried
data too early. The latter was fixed with purpose metadata at checker transport
sites; tests now pass that distinction. Initial clippy had three mechanical
findings, edits applied but final rerun unverified at pause. Successor must read
exact final receipt/source IDs and rerun affected checks, not assume frozen partial
matches the earlier passing subset. No production edits after pause.

After fix-row-provenance lands and full main passes, a FRESH successor duplicates
the preserved draft into its own workspace and rebases, resolving overlapping
check/expr.rs changes to keep both general origin propagation and transport marks.
Never resume superseded role-authority or discard the original failing evidence.

### Preserved provider draft

Author ENDED with frozen UNLANDEDc4f0391df818e1638d9dd14236da87456c40e737
on999926a86bf4e97b579d5c8de4ad303dab36350c.14files/2081insertions11deletions;
all2583input hashes stable.36/37 focused tests, unchanged fix target failure;
fmt/house/size0. Initial clippy3mechanical findings corrected but rerun NOTRUN.
Fullbranch/review/main notrun; draft is NOT complete or landable. Preserve local
receipt-permission-requirements.md,15linehandoff and sibling workspace target/
permission-requirements-prerequisite-evidence with complete diff/log/input hashes.
Fresh successor after verified shared checker fix must retain failing witness
and inspect unverified write::parallel211-234 Fail Bottom handling, which may
overcharge old-row origins on failing upsert branches. No production changes
after pause, no Rust session remains. No precision/safety fix claimed there.

## 2026-10-28: successor dispatched on the repaired checker

The checker repair landed as 1faf0e12 and main passed its full check. A fresh writer owns workspace permission-successor (/home/tony/code/adtb-permission-successor). Its working-copy commit duplicates the preserved draft c4f0391d onto main 27b77539, and it applied without conflicts. The original draft workspace permission-requirements stays untouched as evidence.

Remaining, in order:
1. The draft's update-over-fix test must pass unchanged.
2. Keep the transport marks beside the repair's origin propagation.
3. Settle the suspected old-row overcharge in write::parallel's Fail/Bottom handling with a paired test.
4. Finish the direct Query/Txn, aggregate external-read and upsert precision acceptance.
5. Rerun strict adtb-lang clippy, fmt, house and size.
6. Run one frozen full branch check.
Then a fresh independent review, the lead's landing, and a main check. Local brief: work/briefs/brief-permission-successor.md.

## 2026-10-28: successor frozen; lead ruling on matching an upsert's old row

The successor froze 73e5750339f374fd344d5f197f1ac475739c072c on main 27b77539:
- Full check passed: root 2481 passed, 8 skipped; campaign 47 passed, 1 skipped.
- adtb-lang 718/718 and permission 43/43 passed. Strict adtb-lang clippy, fmt, house-check and size-check passed.
- The update-over-fix test passes unchanged.
- write::parallel overcharged a failing upsert branch, as suspected; confirmed with a paired witness and fixed.
- Receipt work/briefs/receipt-permission-successor.md (local); log target/check-frozen.log in the workspace.

Ruling: a `match` on an upsert's `old` binder charges ReadConstructor on every constructor its arms name, as ranging over those constructors would. Which arm runs is observable through the write, the result or a failure, so it reveals old's constructor. The implicit conflict lookup charges nothing by itself only because it reveals no more than a uniqueness refusal would, and a constructor test reveals more. This applies the existing rule that a role's matches range only over the constructors it is granted; no new normative behaviour. The frozen test upsert_constructor_arms_charge_only_their_own_edges pins the opposite and must change: keep its retag-edge assertions and add the ReadConstructor requirements.

Next: a fresh writer applies the ruling with paired tests; then one full branch check, a fresh independent review, landing and a main check.

## 2026-10-28: old-match ruling implemented; ruling extended to every constructor observation

Frozen 1c8989cc10506f38eaf0252a749df1dc710a0220 on main 27b77539:
- Full check passed: root 2483 passed, 8 skipped; campaign 47 passed, 1 skipped.
- Permission 45/45 and adtb-lang 720/720 passed; strict clippy, fmt, house-check and size-check passed.
- Rows now carry the relations they were read from (`Value.stored`). A `case` on a stored row charges each arm's constructor and its key reads through the existing range path.
- Field and whole-row uses of `old` already charged what ranging does.
- Receipt work/briefs/receipt-permission-oldmatch.md (local).

Ruling: every observation of a stored row's constructor charges ReadConstructor and the key reads on the constructors it can distinguish, as ranging does. This covers `match`, `tag` and any other constructor test, on an upsert's `old` or any stored row. `tag(old)` charged nothing at 1c8989cc and is fixed before landing.

## 2026-10-28: independent review blocks 34e5e03f

A fresh reviewer read 27b77539..34e5e03f (report work/briefs/review-permission-requirements.md, local) and found two under-charges:

- B1: set deduplication is never charged. Sets merge equal elements by whole value, so `count(x for x in left union right)` and `count(y for y in (from x in left select Row { x with id: 0 }))` depend on fields the provider does not charge. The same holds for sum over a union, user aggregates, count over group rows and `top n` ties. Fix: mark rows read from one relation as key-distinct, keep the mark through a join only when both sides carry it, and make aggregates and `top` consume elements without the mark.
- B2: the copy tokens in shape.rs are shared by every row of one range evaluation, so an update through a partial let alias can overwrite `secret` with another row's value and still look unchanged. UpdateField is then not charged. Fix: mint fresh field tokens on the bound old row in `replace`, keeping the row-level token that upsert bindings use.

Also fixed in the same track:
- N1: a whole-row comparison misses the constructor read of a fieldless constructor.
- N2: two tests compare provider calls with each other instead of asserting exact sets.
- N7: `upsert` takes unused parameters.
- N3: test gaps for top/present take and after, at, try, key-of, transitions, and deref through a restricted key.

The precision gaps N4, N5 and N6 are in permission-provider-follow-ups. Everything else on the review list held. house-check passed.

## 2026-10-28: interrupted by a machine crash

The review-fix writer froze 68a9625decead714aa953e55e66acac77d6b4f35 (from 34e5e03f, main 27b77539) and started its full check. The machine crashed during that check (log target/check-fixes.log stops after clippy), and the writer's receipt was lost; its handoff work/briefs/handoff-permission-review-fixes.md (local) survived. The lead reruns the full check on 68a9625d, and a fresh reviewer reads 34e5e03f..68a9625d without builds.
