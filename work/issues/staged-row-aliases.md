---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 798
summary: a transaction let alias loses proposed-row provenance and can be used as a snapshot update or delete target
branch: fix-row-provenance
tags:
  - lang
  - engine
  - test
---

# Staged row aliases

## Report

# Transaction aliases launder write-produced rows into snapshot targets

## Report

The fix-row-provenance author found this accepted during negative regression
work, although writes3.1/5 explicitly prohibit update/delete over write-produced
rows:

```adt
transact
  let staged = insert left from left
  let rows = fix rows = staged union rows in rows
  update x in rows set x
```

Schema is app.Row{id:I64,public:Text,secret:Text}, relation left:Row key id, as
fix-row-provenance witness. check/write.rs::item_heads LetExpr binds a fresh
variable preserving Out.rows but drops mentions_proposed(o.tm); later ranged
sees the alias Var without the nested staged source. Current source proof and
new-repair-tree acceptance establish metadata loss. A tests-only main87527a90
direct-alias executable witness is requested to distinguish preexisting unsafe
acceptance from new fix-query admission; no main executable result claimed yet.

## 2026-10-28: shared checker safety prerequisite

Same fix-row-provenance agent/workspace owns this tightly coupled alias metadata
repair. Preserve proposed metadata with ordinary LetExpr snapshot bookkeeping;
audit write-match/query/upsert alias binders for same loss. No relaxed ranged
veto, Fix syntax exception, data inference, dependency or normative change.
Retain direct/multi-hop/recursive update+delete refusal tests, ordinary snapshot
alias positive, existing snapshot/upsert controls, and failed baselines. Do not
broaden existing taint semantics while fixing loss. Main full branch/fresh source
review/root fullmain gates are shared with fix-row-provenance. Root schedules
Rust builds. Permissions draft remains paused until shared safety repair verifies.

### Original-main safety failure confirmed

Tests-only839d0e27/base87527a90 with original production source fails both
direct-alias update/delete rejection tests: checker ACCEPTED both. nextest100,
2failed/0passed,0.008s. Retained provenance-alias-baseline.log, manifests and
result/timing JSON in engine workspace target. Candidate production backed up
with SHA manifest before temporary original-source run, then all six files
including rows.rs restored byte-for-byte; all baseline process exits collected.
No candidate fix claim follows from baseline. First candidate all-lang650/650
passed; expanded a50f772c with ordinary aliases/upsert controls and direct
fn_rows reuse is still under focused checks. Public API regression written,
not yet built at this checkpoint. Simulator slot released after lang/clippy;
41GBfree, notify35/guard30. Fresh review/fullbranch/main still required.

## 2026-10-28: frozen for user-requested session restart

Author candidate1ed18a4b1be6a2a502c40e9d4787cc3291d4e444/base87527a90
in /home/tony/code/adtb-fix-row-provenance is UNLANDED.654/654 lang unit
tests, strict langclippy, fmt/house/size all exit0. Last authorized public API
check `suite -p adtb-sim --test api fixpoint_write_targets_preserve_the_snapshot_relation`
collected0:1pass/47filtered,0.173s; build1m28s. Asserts exact identity update,
changed-field mutual fix update with key predicate, filtered recursive delete,
correct survivor fields and untouched other relation of SAME row type/key.
2578tracked/localconfig hashes recorded; final after-hashes in author receipt.

No fullbranch, broader simulator gates, fresh independent review, landing or
lead main integration claimed. All remain REQUIRED after restart; user requested
wind-down, no new review/feature tracks started. Root keeps tickets in-progress
on existing branch. Author saves local main work/briefs/receipt-fix-row-provenance.md
and <=15line handoff-fix-row-provenance.md, raw target/provenance-* logs/manifests.
Original failing baselines and corrected fixture/lint failures remain retained.
No Rust process after final gate;41GBfree/own target2GB, no cache cleanup.
Fresh provider successor still waits for verified shared repair, preserves old
c4f0391d draft and original witness. Never resume ended author for followup.
