---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 60
summary: "nothing shows the incremental checks a constraint is admitted with, so adtb-probe --explain cannot print them"
closed: 2026-10-29
tags:
  - tooling
  - devex
---

# Feature: show a constraint's incremental checks

## Report

`adtb-probe --explain` prints the engine's plan for a query. For a constraint it should also print the incremental checks the constraint is admitted with, but `write/delta::compile` is internal and `explain` does not show its output, so the probe has nothing to read. Agents have added `eprintln!` calls to `write/delta/mod.rs` to see them.

Fix: an engine surface for the compiled checks of a constraint, for example `explain` over a constraint name printing each check's plan, and `adtb-probe --explain` printing it.

## 2026-10-29: dispatched

A fresh writer owns this in workspace explain-checks on main 93da74f8. Lead ruling: this is developer tooling, so no language syntax and no spec change; `explain` output text is not normative (catalog.md 7.1). The engine gets a Rust call returning the plan node of each compiled incremental check of a named constraint or check, and adtb-probe prints them. Gates: targeted adtb and adtb-probe tests, clippy, fmt, house-check, size-check; the lead runs the full check. Local brief: work/briefs/brief-explain-checks.md.

## 2026-10-29: landed

Landed as bf2c4ff2; the main check passed on it. `adtb::Database::explain_checks(name)` returns a `CheckPlan` per incremental check the constraint or check is admitted with, read from the checks compiled at open, each labelled by its triggering constructor relation and `Part`; an unknown name is an invalid error naming it. `adtb-probe --schema FILE --checks app::NAME` prints them. Follow-ups are in explain-checks-follow-ups.

## 2026-10-29: resolved
