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
