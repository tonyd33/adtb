---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 799
summary: asserted principals are accepted without validating nested value domains
branch: principal-validation
depends-on: [ordered-schema-shared-graph]
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
