---
created-at: 2026-11-14
status: open
kind: feature
priority: 380
summary: "a fresh-key placeholder is a record typed at a non-record type"
tags:
  - lang
  - structure
---

# Feature: a kernel kind for the fresh-key placeholder

## Report

Found by the kernel validator of structure-lang-identity step 8a (a1436ec3), whose hook at this exit was removed until this is fixed.

`kernel-fresh-placeholder-kind` (feature, tag `lang`; the ruling, not a bug: a convention the
kernel cannot state). Summary: "a fresh-key placeholder is a record typed at a non-record type".
Make the placeholder a kernel kind of its own, typed at its type (`Key r` or `Instant`);
`fresh::rewrite` emits it and `fresh::fill` matches that kind, not a `Record` at a non-record
type; then restore the plan-exit hook (`plan/validate.rs`, `assert_valid` over every expression
the walk reads) and the `Stmt::new` hook, with no skipped node. Repro: restore the plan-exit
hook and run `write::write_tests::write_examples_on_several_workers_agree_for_seed_1`; the
placeholder `Record` built at `write/fresh.rs:65-75` fails "a record is not typed by a record".

Blocks structure-lang-identity step 8b-2 (typed constructors in the engine).
