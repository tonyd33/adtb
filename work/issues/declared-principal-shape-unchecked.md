---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "a schema whose Principal lacks Service { name: Text }, Anonymous or EndUser checks, and then every call on it panics"
closed: 2026-10-27
tags:
  - quality
  - lang
  - engine
---

# A schema with a malformed Principal checks, then every call panics

## Report

`check_type_name` at `crates/adtb-lang/src/schema.rs:1177-1188` checks only that `Principal` is declared once. It never checks the three constructors `docs/spec/security.md` requires of it (`Service { name: Text }`, `Anonymous`, `EndUser`).

`Database::principal` at `crates/adtb/src/api/mod.rs:537-541` runs on every call, ad hoc text included, and does `.expect("the schema's `Principal` has a `Service` constructor")`.

Repro: declare

```adt
type Principal = Admin | Guest
```

The schema checks. Then any `query_text` panics. With `Service { name: Int }` the owner value built at `:543` is ill-typed instead.

Fix: have the schema checker refuse a `Principal` without the required constructors and field types, with a schema_tests refusal and a conformance near miss. The `expect` then guards a checked invariant.

## 2026-10-27: done

The schema checker refuses a declared `Principal` that is not a sum, has header fields, or lacks exactly `EndUser`, `Service { name: Text }` and `Anonymous` (`check_principal` in `crates/adtb-lang/src/schema.rs`), with refusals in `schema_tests.rs`.

Still open: no conformance near miss, as the script format has no directive for a schema that must be refused.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the embed track. What stays open is in `embedded-open-follow-ups`, `host-fns-in-keys`, `spec-closed-database-reason` and `conformance-refused-schema`.
