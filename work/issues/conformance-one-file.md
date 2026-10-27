---
created-at: 2026-10-26
status: open
kind: feature
priority: 84
summary: "no command runs one conformance file on the reference and the engine, so agents run a whole part of the suite to check one case"
tags:
  - tooling
  - devex
  - test
---

# Feature: run one conformance file on both sides

## Report

To check one `.test` file, agents in the 2026-10-26 tracks ran a whole part: `cargo nextest run -p adtb-conformance --test engine queries` took 35 to 55 seconds (evo, eadhoc), and `cargo nextest run -p adtb-conformance` 120 to 166 seconds (speed, ref3, which then moved to the background). The engine harness already runs a single file when `ADTB_CONFORMANCE_ENGINE_FILE` is set (`crates/adtb-conformance/tests/engine.rs`), but nothing documents it; eadhoc found it by reading the harness. The reference side has no single-file switch.

Fix: a devenv script `conf FILE... [--engine|--reference]` that runs each named file on the reference and the engine and prints each difference with the file and line of the case, and a line in `conformance/` (or the conformance design file) naming it. A glob or directory argument runs every file under it.
