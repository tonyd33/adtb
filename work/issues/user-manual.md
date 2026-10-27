---
created-at: 2026-10-26
status: open
kind: feature
priority: 880
summary: "adtb has no user manual or reference, so a new user has only the normative spec to learn from"
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
