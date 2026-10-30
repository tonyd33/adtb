---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 880
summary: "adtb has no user manual or reference, so a new user has only the normative spec to learn from"
branch: manual-gaps
tags:
  - docs
---

# Feature: A user manual and reference for adtb

## Report

Tony: a completely new user should be able to read it and understand how to use adtb.

The spec in `docs/spec/` is normative and declarative. It says what holds, not how to get started, and it assumes the reader already knows the model. Write a manual for someone who has never seen adtb, in `docs/manual/`, in four parts:

- Tutorial: from nothing to a working embedded database, in order. Install and open a database, declare a relation-level sum, insert and query rows, match on constructors, add a reference and a constraint, run a transaction, ship a schema change as a migration, pin a bundle, and call it from Rust. One running example throughout, small enough to type out.
- Guides: task-shaped pages for things a user does after the tutorial, such as modelling optional and many-valued facts without NULL or collection fields, choosing keys and indexes, writing constraints the engine admits and reading its refusals, retagging rows, evolving a schema across releases, loading CSV and NDJSON files, and using the shell.
- Concepts: the model in plain terms for readers coming from SQL or document stores. Relations as sums, constructors, identity and references, why there is no NULL, serializable transactions, and how migrations and bundles relate.
- Reference: the language (declarations, queries, writes, migration operators, ad hoc text), the in-process API, the shell commands, options, and every error a user can see with what it means and what to do about it.

Rules:
- The manual must agree with the spec. Where the two differ, the spec wins and the manual is fixed. Reference pages may link to spec sections for the exact rules, but must be readable without them.
- Document only what the engine runs. A feature the spec has and the engine refuses is left out or marked as not yet available, never described as working.
- Every `adt` and Rust example runs. A test in the suite extracts the manual's code blocks and runs them against the engine, checking the output shown on the page.

Done when:
- all four parts exist and cover every user-visible feature of the release;
- the example test passes in the default run, within the test-time budget;
- a reader check passes: a fresh agent given only `docs/manual/`, with no access to the spec or the code, models a small domain, writes its queries, constraints and one migration, and the result runs; every point where it got stuck is fixed in the manual;
- Tony reads it as a new user and signs off.

## 2026-10-26: done

Landed: the manual under `docs/manual/` in four parts. Tutorial: getting started and seven pages (schema, sums, queries, match, references and constraints, transactions, migrations) on one running example. Guides: modelling with sums, evolving a schema, writing constraints, files and CSV scans. Concepts: relations as sums, no NULL, keys and references, serializable transactions, the migration log. Reference: the shell, the language, the prelude. `crates/adtb-shell/tests/manual.rs` runs every page's sessions, with stdout and stderr interleaved, continuation prompts, `< FILE` input and `adt fragment` excerpts skipped.

Open: the in-process Rust API page and its example test; the errors reference; bundles and pinning, and the operational commands, which the engine refuses; `script`, `add default`, `add computed` and `store`, which it refuses at apply. The README lists these as planned pages. The reader check and Tony's sign-off have not happened. Under load the migration pages run near 15 s; quiet they take about 2 s.

## 2026-10-27: landed

The tutorial, guides, concepts and reference landed in `docs/manual/`, every example run by `crates/adtb-shell/tests/manual.rs`. Still open: the Rust API page (needs a test that compiles Rust examples), the errors reference, the pages for operators and commands the engine refuses (listed under "Not yet in this manual" in `docs/manual/README.md`), a reader check, and Tony's sign-off. Bugs found are in `manual-found-panics`, `after-literal-unchecked`, `keyed-record-add-ctor`, `refusal-message-follow-ups` and `shell-new-user-follow-ups`.

## 2026-10-27: done

Landed: `docs/manual/reference/rust-api.md` (crate setup, migrations, a bundle and its hash, code generation in `build.rs`, open and call, values, calls by ordinal through `Registered`, ad hoc text, transactions and `transact`, cancellation and deadlines, Arrow results, bulk append, host functions, open options, close) and `docs/manual/reference/errors.md` (every reason the engine raises, with cause and remedy, open errors, and the reasons it does not raise). The new crate `crates/adtb-manual` extracts each page's `rust` blocks into tests: each block is a whole `main.rs`, run in an empty directory, its printed lines compared with the `text` block after it; the page's `build.rs` block is the build script the crate runs, checked byte for byte. A reader pass fixed contradictions and gaps in the language reference, tutorial 1, 5, 6 and 7, the guides, the migration-log concept and the prelude, whose sessions now show each output under its input. The README lists what is still missing: bundles in the shell, translation of earlier pins, reading the past, the rest of the in-process API, operations, and the operators refused at apply.

Open: the reader check by a fresh agent writing a program from the manual alone, and Tony's sign-off.
