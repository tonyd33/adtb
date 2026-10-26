---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 696
summary: "every insert into a relation with an integer key advances a durable counter, though only sequence keys have one"
closed: 2026-10-25
tags:
  - quality
  - engine
---

# Bug: unsequenced integer keys advance a durable counter

## Report

`Stmt::apply` (crates/adtb/src/write/apply.rs:189-196) calls `advance_sequence` for every inserted row whose key is a `Val::Int`, whatever the relation's generator. docs/design/mvcc.md §12.10 gives a counter only to a `sequence` key, and `Database::new` (write/mod.rs) seeds counters only for `Generator::Sequence` relations.

Effects: each relation with an integer key gets a counter on the sequences page; `Sequences::advance` (write/sequence.rs:98) sets `reserved = next` with no block, so every insert of a new largest key writes the page and forces the log under the sequences lock; and once a schema has more integer-keyed relations than the page holds (about 339 at 4 KiB, write/sequence.rs:118), statements fail with `Unsupported("more than N generated counters")` though they generate nothing.

Fix: advance only when `r.generate == Some(Generator::Sequence) && r.single_key`. Consider reserving a block past `held` on advance too, so a run of explicit keys does not force the log per row.

## 2026-10-25: resolved

## 2026-10-25: resolution

Only a `sequence` key advances a counter; `only_a_sequence_key_has_a_counter` saw 2 counters on the old code.
