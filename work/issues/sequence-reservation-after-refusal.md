---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 699
summary: a refused sequence reservation can be returned successfully without durable coverage on retry
branch: sequence-repair
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
sequence-repair at /home/tony/code/adtb-sequence-repair on5f29e1cc isolates
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
command from role workspace: direnv exec /home/tony/code/adtb env
DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" cargo nextest run -p adtb
--lib write::sequence::tests. Retained logs target/sequence-baseline-regressions.
{log,status}, source write/sequence_tests.rs. Fresh repair worker independently
confirms before fix. Role/provider and campaign remain paused.
