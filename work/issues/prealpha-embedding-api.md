---
created-at: 2026-10-25
status: wontfix
kind: feature
priority: 260
summary: "there is no in-process API an application can call; Database, ad hoc text, transact and typed results do not exist"
closed: 2026-10-25
depends-on:
  - reference-interpreter-writes
  - prealpha-schema-from-declarations
tags:
  - prealpha
  - engine
---

# Feature: In-process API for the interpreter-backed pre-alpha

## Report

The pre-alpha lets an application use adtb before the engine exists: the in-process API of `docs/spec/embedding.md` (§1, §2 without migrations, §5, §6: `Database::open`, ad hoc text, `transact`, interactive transactions, retry, typed result values) running on `adtb-lang` and `adtb-reference`. Rulings: the API lives in `adtb`, the crate the engine will implement, over an internal backend contract written in a new `docs/design/embedding.md`; the pre-alpha backend wraps `adtb-reference` behind a default cargo feature `interpreter`, removed when the engine backend lands; the reference interpreter stays an independent oracle (no dependency from it on `adtb`). Transactions run serially under one lock; the API shape is the spec's, not the backend's.

## 2026-10-25: wontfix

Dropped: Tony decided the first application waits until the engine is ready, so no interpreter-backed pre-alpha is built.
