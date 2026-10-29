---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 799
summary: registered calls accept values outside their declared parameter representation
branch: registered-admission
tags:
  - engine
  - test
---

# Registered argument values

## Report

At main39dc440e, api/registered.rs::prepared checks only argument count before
cloning supplied values into the checked program. Entry::types already preserves
every declared parameter type, including unused parameters whose variable is
None. The shared api/value.rs::valid validator is used for Principal assertions
but not registered arguments. The broader registered-calls-follow-ups ticket
records this gap; this ticket owns the concrete repair. This is source evidence;
an executable failing baseline remains required before claiming a reproduced case.

## 2026-10-28: shared admission track

Fresh workspace registered-admission starts8ff92c6c on39dc440e. One writer owns
api/registered.rs, narrow API-boundary helper integration and focused simulator
tests. Reuse shared representation validation at the registration's checked schema,
for every declared argument including unused ones. Preserve scalar ranges, exact
arity, nested/generic constructor restrictions, collection constraints and Unicode
rules; newtype predicates remain write-only. No printer/roundtrip validator or
transport-only fix, and no nominal identity inferred from an untagged Value.

The same call-admission boundary owns bundle-invalid-call-renews: all pre-execution
checks, including principal/arity/value and interactive read-only mode, precede
renewal. Preserve positive renewal and lifecycle/current-pin errors. Do not widen
the scope to lease locking or runtime failures without evidence. No new dependencies
or spec choice. Raise any concrete contract ambiguity to lead, never alter spec
to accommodate implementation. Read-only authority-next-audit may run alongside;
no permission/binding implementation branches until this provider lands.

Acceptance: retained failing baseline; standalone, Arrow and interactive call
coverage; invalid arguments including unused parameters refuse without execution,
lease change or write; valid arguments including write-only-domain newtypes remain
valid. Rejected interactive call leaves transaction usable under its existing
contract. Focused registered/bundles and relevant shared-value tests, strict
affected clippy/fmt/house/size, full frozen branch check, fresh independent review,
lead landing and full main check. Record exact source/configuration hashes,
commands/status/logs/timings and unresolved failures. Broad follow-ups remain open.
