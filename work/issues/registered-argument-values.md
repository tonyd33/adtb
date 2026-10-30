---
created-at: 2026-10-28
status: resolved
kind: bug
priority: 799
summary: registered calls accept values outside their declared parameter representation
closed: 2026-10-28
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

## 2026-10-28: baseline reproduced and preflight audit

Author retained tests-only ca493bf0cbd82cf7c8209a93d7990e37d54bcdc5, nextest exit100:
an unused I8 parameter supplied as128 returned Ok(Committed Int1), and wrong
arity after four simulated hours advanced the bundle's renewed timestamp from
00:00:00.135841 to04:00:00.147272. Logs/source copies are in worker
target/admission-baseline*. This confirms the initially source-inferred cases.
Final shared validator/admission implementation and all checks remain in progress.

Lead source audit additionally identified preflight stop/snapshot/state and
engine-close checks occurring after current renewal: Engine::call checks stops
and reads_after, while AdHocTransaction::run checks engine-open/state and merged
transaction/call stops. Author must assess/reproduce those refusal paths before
claiming complete pre-execution admission ordering. Source-inferred close race:
a cloned Database can close the pager while an interactive handle survives, and
registered renewal currently precedes run's enter guard. No crash reproduction
is claimed yet. Keep checks/renewal under a single owning engine guard; nested
read guards can deadlock against a waiting close writer. No pager workaround or
new promise about failures after execution is authorized by this clarification.

## 2026-10-28: resolved

Landed `744bddb0592a6a3fdbf01bca00e2f02149b0c6c1`, `fix: validate calls before
lease renewal`. All declared registered argument positions use the existing
api/value validator against Checked.schema, including unused/imported parameters.
Invalid values refuse before cloning, execution or renewal. Representation checks
preserve write-only newtype domains, short UUID values and key semantics.

Fresh independent reviewer cleared all five code/test paths and all 2,568 frozen
input hashes on `a5b883281e07474bc8373dc345d2b9c20deabb09`/base39dc440e.
All 173 affected integration tests, four validator tests, strict lint/fmt/house/
size and full branch check passed. Branch full collected0, 418seconds,
2026-10-29T00:35:12Z–00:42:10Z: 2,426 root passed/eight skipped (52.502s),
47 campaign passed/one skipped (20.879s), all static/docs checks passed.
Original true baseline failures and fixture-only failures remain distinguished.

Full main check collected0 on clean `4b5eec71ca4a1631e2d965172e3e471841b33c2c`,
parent744bddb0, all tracked/local root/app Cargo inputs and revision unchanged.
From ../adtb:

    direnv exec ../adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check

2026-10-29T00:44:58Z–00:49:08Z (250seconds, one-second precision).
All 2,426 root tests passed/eight skipped (49.447s); 47 campaign passed/one
skipped (20.290s); lint/fmt/parity/dependency/size/docs passed. Docs: ten tests
in10.255s,420syntax inputs,61pages,6,734links,4,999fragments,zero failures.
Session14135 collected0. Permission/queue/execution-only timings unknown.
Log target/registered-admission-integration/check.log SHA256:
`627e1dec5b204ef4c6a9b12f655ad42e1a0f0d585e4d12e4cfe7594862cb30bb`.

Tracked receipts/review/handoffs: work/briefs/*registered-admission*.md. Landing
adds only ten metadata/evidence paths relative to the reviewed candidate; all
five source paths match. No spec, dependency or public API export changed.
Broader registered-call limits/feed/aggregate features remain open separately.

## 2026-10-28: resolved
