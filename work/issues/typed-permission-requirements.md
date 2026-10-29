---
created-at: 2026-10-28
status: open
kind: feature
priority: 829
summary: no typed provider extracts and checks complete relation/field/write privilege requirements
depends-on:
  - registered-argument-values
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
