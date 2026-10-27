---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 840
summary: no generator emits typed Rust calls from a bundle file
closed: 2026-10-27
depends-on:
  - bundles
  - embedded-open
tags:
  - engine
  - lang
---

# Feature: Rust code generation from a bundle file

## Report

Implement `docs/design/kernel.md` §8: Rust code generation from a
bundle file, typed calls over the in-process API. Settle `kernel.md:398`,
the API by which `adtb-lang` gives a generator ids and ordinals.

Prerequisites: `bundles.md`, `embedded-open.md`.

## 2026-10-27: done

`adtb_lang::signature` gives a generator what registration assigns: `replay(log)` is the source a database holds after the log, its `Schema::ids` the catalog ids, and `bundle(source, text)` the callables of a bundle file in ordinal order with their parameter and result types; `kernel.md` §8 states it. Registration now shares its module imports and bundle functions with it. `crates/adtb-codegen` is the generator: `generate(log, bundles)` for a `build.rs` and an `adtb-codegen --log DIR --bundle MODULE=FILE` binary. It emits `schema::LOG`, the host types of every user type and relation variable with their type, constructor and relation ids, and per bundle `BUNDLE`, `Error` and one value per callable. `adtb::typed` holds the typed API it calls (`server.md` §4.5): `Database::bundle`, `Bundle::{call, call_arrow, begin, transact}`, `Transaction`, `Rows`, `Pending`, `Key<R>`, and `Load::appender` now takes a `CtorRelation`. `crates/adtb-codegen-tests` compiles the code of a fixture shop and of `apps/campaign` and calls the shop's over the simulator against ad hoc text. Deviations from `embedding.md`: `Transaction` and `Bundle` carry the runtime and a lifetime, `call_arrow` returns the untyped `ArrowStream`, and a query whose result is no set returns the value itself.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the codegen track: `adtb_lang::signature`, `crates/adtb-codegen`, `adtb::typed`, and `crates/adtb-codegen-tests`. Follow-ups are in `codegen-follow-ups`, `spec-codegen-gaps` and `campaign-codegen-port`.
