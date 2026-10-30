---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 699
summary: a refused sequence reservation can be returned successfully without durable coverage on retry
closed: 2026-10-27
---

# Sequence reservation after refusal

## Report

Sequences::draw and advance publish next/reserved in their in-memory map before
persist succeeds. persist can refuse nonfatally when counter count exceeds the
single sequence-page capacity. Retrying the same counter may skip persist using
the in-memory reserved bound and return a value without durable coverage. Pager
failure/overflow variants require audit too. Initially a code-evidenced suspected
engine invariant violation; capture executable baseline failure before claiming
confirmation. No app/role workaround.

## 2026-10-28: engine-first reproduction and repair

Campaign-load and role-authority expansion paused. Role agent prepares minimal
baseline counter-capacity witness and preserves provider source. Fresh worker
sequence-repair at ../adtb-sequence-repair on5f29e1cc isolates
sequence semantics from role authority. Detailed brief brief-sequence-repair.md.
Preserve failed regression/log. Cover draw and advance, existing/new counter,
room/overflow/persist failure, unaffected counters, successful reservation/
amortization and crash/reopen identity monotonicity. Pager fsync failures are
fatal, not safely retried. No dependency or spec-promise changes.

Acceptance: failing baseline regression then general fix, exact independent
review, affected sequence/simulator tests + strict clippy/fmt/house, full branch
and main check with stable inputs/final exits/receipts. After verified repair,
rebase fresh campaign/provider successors and resume their saved acceptance.

## 2026-10-28: baseline confirmed

Role-authority private real-pager witnesses, baseline sequence.rs plus test-module
include, failed all3 with nextest exit100 in0.055s (compile52.24s). Capacity retry
draw returned Ok(2) for an ID absent from the durable page; advance retry returned
Ok(()); draw floor i64::MAX panicked at sequence.rs89 arithmetic overflow. Exact
command from role workspace: direnv exec ../adtb env
DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" cargo nextest run -p adtb
--lib write::sequence::tests. Retained logs target/sequence-baseline-regressions.
{log,status}, source write/sequence_tests.rs. Fresh repair worker independently
confirms before fix. Role/provider and campaign remain paused.

## 2026-10-28: boundary behavior and error ownership

Preserve explicit i64::MAX key inserts: stored MAX is an exhausted candidate
sentinel, no format change. Generated draws refuse without mutation/panic and
remain exhausted after reopen. Do not reject the valid explicit-key domain to
avoid arithmetic. Add exact API and crash/regression tests. Exhaustion is real,
not an unimplemented form: minimal internal sequence-exhaustion error in
write/mod.rs plus api/error.rs conversion to existing API Invalid approved;
no new normative error constructor. Existing counter-capacity limitation remains
separate from correct failure atomicity. No concurrent shared API writer.

## 2026-10-28: resolved

Landed96744632, fix: preserve sequence reservations on refusal. Candidate Counter
state is copied, checked and rolled back completely on persist error (remove
new entries too), so refusal cannot suppress a later durable reservation. Checked
draw successor returns specific SequenceExhausted, mapped truthfully to existing
API Invalid. ExplicitMAX keys remain legal with durable exhausted sentinel;
page format and successful reservation amortization unchanged. No app/role
workaround, dependency, normative spec or other design changes.

Exact independent review38867188a4defc432d43869b8bcc24a877b40949 on
0568954960368a17a4dba2f17c67b45222a5bb67 found no actionable blocker. Seven
unit tests cover repeated capacity/read-page errors, exact map/page preservation,
unaffected counters, overflow/explicitMAX, lower bounds, force amortization and
actual fault-backed crash/WAL replay. API test preserves explicitMAX row and
checks exact exhaustion refusal across two reopens; existing sim crash/non-reissue
test also passes. New units0.173–0.352s each/API0.930s in full suite. No exhaustive
schedule sweep or680relation API fixture; no new reversible fsync-failure claim.

Branch authoritative check0,404.305s,04:02:05.831693Z–04:08:49.945706Z; all
tracked/localCargo hashes and revision unchanged.2357root/8skip,46campaign/1skip,
allclippy/fmt/lintparity/deny/size/docs0. Docs8fixtures/420syntax/61pages/6730links/
4995fragments0fail. LogSHA256f01230bf674e3ac24d3c79772e8139815a56d23b3921f4342a5a07313872252b.
Lead verified landed tree identical to38867188, then authoritative main check0,
2026-10-28T04:10:28Z–04:15:34Z,306s, same counts/allchecks. Clean tested WC
86f0fd9e3b134f96dddeca6ceab58c30b0704873 parent96744632; tracked/localconfig
hashes and revision unchanged before/after. Command main root: direnv exec
../adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check.
Rust1.98.1/cargo1.98.0/defaultfeatures/profiles; permission/queue/execution-only
unknown. Main logSHA25630d431991bc034e0306b6390e84ab869d969466bbdf2c295b25619e148e590e9.
Evidence target/sequence-repair-integration/, target/sequence-repair-evidence/;
receipt/handoff-sequence-repair.md and independent review in work/briefs.
Earlier failed baseline/oracle/lint logs retained, not erased by successful runs.

All engine processes ended. Preserve source/logs/fixtures; only completed sibling
target/debug removed after verified evidence copies for disk headroom. Paused
campaign/provider may now resume with fresh agents on this general repair.

## 2026-10-27: resolved
