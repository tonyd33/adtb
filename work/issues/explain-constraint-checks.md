---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 60
summary: "nothing shows the incremental checks a constraint is admitted with, so adtb-probe --explain cannot print them"
branch: explain-checks
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
