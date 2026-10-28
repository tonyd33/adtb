---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 799
summary: asserted principals are accepted without validating nested value domains
branch: principal-validation
depends-on: [ordered-schema-shared-graph, campaign-live-fixture-lifecycle]
tags:
  - engine
  - quality
  - test
---

# Asserted principal value domain

## Report

Source-inferred at main c14a6594: crates/adtb/src/api/mod.rs Engine::principal
checks only the outer Principal constructor/arity, then clones its fields. A
custom Principal EndUser { level:I8 } can receive Value::Int(256) through
CallOptions.assert; the engine should refuse typed Invalid before execution.
The role provider's draft textual printer check cannot validate it either:
textual::print accepts any i64 at narrow integer form and may omit nested
nullary constructor field validation. No executable main witness yet.

Fresh principal-validation workspace, base c14a6594, owns minimal public engine
regression and general shared typed-principal boundary fix. First prove baseline;
reuse existing value/domain facilities, never print/parse roundtrip or unsafe
storage-conversion validation. Audit recursive shapes, scalar domains and newtype
constraints; return plan before significant implementation. No new dependencies
or normative choice approved. Engine-first pauses unlanded role-authority; this
repair lands before a fresh provider successor rebases. Campaign remains landed.

Acceptance: invalid nested/range assertions refused exactly, valid and owner/
Service controls preserved across shared embedded/registered/interactive paths;
focused strict clippy/fmt/house, fresh independent review, authoritative branch
check and lead full main check. Build only own target, serialize cold builds and
keep30GBfree. Exact IDs/hashes/commands/exits/logs/limits in receipts. Separate
registered-calls-follow-ups argument integration remains open and unclaimed.

## 2026-10-28: refinement scope

Adopted constraint/refine/write-only and constraints.md2.3 require newtype domain
predicates only when values are stored in relations. This in-memory boundary
validates their representations and must not evaluate those predicates. Add an
accepted in-memory control that violates such a write-only predicate. Preserve
all existing write-domain checks. Scalar range/domain representation checks,
recursive arity and constructor restrictions remain in scope. No new policy ruling.

## 2026-10-28: public baseline confirmed and shared plan

Main-based source9d3e76bf/basec14a6594 reproduced both regressions through every
shared entry: ad hoc query, registered call, and interactive Begin each returned
Ok for I8 value256 and a Bool nullary constructor carrying an extra field. Exact
nextest exit100, test execution0.144s/total3.105s, tracked/local config hashes and
revision stable. Earlier cold fixture run lacked its import and is retained as
setup failure, not bug evidence. Logs under principal-validation/target/principal-
baseline; the witness leaves existing Engine::principal unchanged.

Approved bounded implementation: one internal API value validator over Schema,
Ty and Val using safe iterative traversal and memoization for shared subvalues;
reuse existing constructor/key-layout/type/range/calendar facilities. Shared
Engine::principal keeps Service Denied precedence, validates before cloning, and
serves all three entry points. No role-only checks or printer roundtrip. Existing
write-only refinement predicates remain outside in-memory type validation.

The audit also found Val::text merely wraps Arc text. The same track may make
that standard constructor normalize to NFC with existing Unicode support and
no-extra-allocation already-NFC fast path; raw enum bypasses are checked at the
boundary. Add direct constructor and valid/raw principal controls, simplifying
redundant existing Text normalization where directly affected. This is limited
execution-value construction scope, not a broad Unicode or argument refactor.

## 2026-10-28: ordered checker prerequisite, no test bypass

First complete validator source caab9c7036b6e4d392490266dc7afc50dde7c242 on
c14a6594 has the shared iterative validator, bounded pair-memoized Span order
check, constructor NFC fast path and reused decimal predicate. Fresh exact
source review has no concrete blocker so far, but verification is NOT complete.
The affected runner stopped100 after three unit tests passed and its legitimate
30-level ordered graph fixture timed out60.004s; total87.219s includes compilation,
source/config hashes and revision stayed unchanged. Later gates did not run.

Source audit identifies Schema builder check_ordered before the first valid()
call, recursively unfolding both fields of every shared predecessor. Exact stage
isolation/general repair now belongs to ordered-schema-shared-graph. Do not
bypass checking with manually trusted ordered flags, reduce depth to hide the
case, raise timeout or count three scoped passes as final. Original fixture and
timeout are preserved. Source-inferred finite generic and newtype order cases
are acceptance candidates for the language repair, not yet proven bugs.

Landing order: language repair and full main integration first; then this worker
rebases frozen caab9c70 and reruns the unchanged legitimate DAG test, all affected
gates, exact review/full branch and lead main integration. Role provider remains
paused. Fresh language workspace owns sole cold build slot; principal has no
active build/check process. All baseline/setup/compile/timeout receipts remain in
work/briefs/receipt-principal-validation.md and original target logs.

## 2026-10-28: correction to no-import baseline diagnosis

The earlier statement that afd90d40 merely lacked a required fixture import was
too quick. Security1.1 and adopted spec/schema-principal/root-first make that
no-import bundle valid. Its RegistrationRefused expectedPrincipal/foundPrincipal
is separate public engine lookup evidence, tracked in custom-principal-global-
lookup. It still does not prove the value-validation hole because it fails at
registration first. The imported9d3e76bf fixture is valid and independently proves
the nested assertion bug; this repair must not claim root lookup fixed. Original
logs and source IDs are retained; current fullcheck stays frozen.

## 2026-10-28: full verification blocked by existing campaign fixture

After verified ordering repair928b7d38, principal rebased cleanly to
f987476fc777ed7aafdf23fc18e8e8e28137cbee; both test files byte-identical to caab9c70.
All six focused commands passed0 with stable inputs/revision,115.612579s:
4generic units (unchanged DAG0.010s),10public principal tests,29registered tests,
strict adtb+sim all-target clippy/fmt/house. Fresh exact8path independent source
and focused review clear; no fullpass claimed.

Authoritative fullcheck f987476f collected100/stable,406.417s: all2,387root tests
passed/8skipped51.146s; campaign46/47passed with one existing live test failed.
The failure is a confirmed test lifecycle leak, tracked in campaign-live-fixture-
lifecycle; engine correctly refused a second proposed Underway for the same
session. Later policy/size/docs/house unfinished. Keep this failed run and both
original code/test files unchanged. Fresh isolated fixture repair lands/mainchecks
first, then principal rebases and reruns full branch before landing/maincheck.
No role/campaign feature expansion, forced schedule or error suppression.
