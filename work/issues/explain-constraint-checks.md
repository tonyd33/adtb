---
created-at: 2026-10-26
status: open
kind: feature
priority: 60
summary: "nothing shows the incremental checks a constraint is admitted with, so adtb-probe --explain cannot print them"
tags:
  - tooling
  - devex
---

# Feature: show a constraint's incremental checks

## Report

`adtb-probe --explain` prints the engine's plan for a query. For a constraint it should also print the incremental checks the constraint is admitted with, but `write/delta::compile` is internal and `explain` does not show its output, so the probe has nothing to read. Agents have added `eprintln!` calls to `write/delta/mod.rs` to see them.

Fix: an engine surface for the compiled checks of a constraint, for example `explain` over a constraint name printing each check's plan, and `adtb-probe --explain` printing it.
