---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 799
summary: asserted principals are accepted without validating nested value domains
branch: principal-validation
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
