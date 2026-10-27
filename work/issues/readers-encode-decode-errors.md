---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 859
summary: "the reader rules build drop operators as migration text and parse it back, and Debug-print the checker's diagnostics into Error::Internal"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# Releasing a held drop formats migration text and reparses it, with errors as Debug strings

## Report

`without` at `crates/adtb/src/catalog/readers.rs:658-673` builds each pending drop as migration text with `format!` and parses it back to apply it. `checked` and `without` (`:677-683`) map the checker's diagnostics into `Error::Internal(format!("{e:?}"))`. A name that needs quoting breaks the round trip, and a failure reads as nested Debug text.

Fix: build the drop operators directly (the `MigrationFile` operator values), and keep the diagnostics typed in the error.

## 2026-10-27: done

`without` builds each drop as an `Operator` value (`OperatorKind::DropConstraint` or `DropCheck` over a `QName`) and applies it with `Source::apply`; no migration text is formatted or parsed. Failures are the new `catalog::Error::Unchecked(String, Vec<Diag>)`, holding what was built and the checker's diagnostics; `checked` returns `Vec<Diag>`, and `held` reports each diagnostic's message in its refusal. `api::migrate` maps `Unchecked` to `RejectReason::Internal`. No test: no constraint name needs quoting in this grammar, and the failure path needs the checker to refuse a schema with only constraints dropped.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
