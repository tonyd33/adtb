---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "a schema whose Principal lacks Service { name: Text }, Anonymous or EndUser checks, and then every call on it panics"
tags: [quality, lang, engine]
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
