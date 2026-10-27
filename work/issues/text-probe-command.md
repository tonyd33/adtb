---
created-at: 2026-10-26
status: open
kind: feature
priority: 89
summary: "no command checks a schema and a text and runs it on the reference and the engine side by side, so agents write throwaway tests to try a query"
tags:
  - tooling
  - devex
---

# Feature: try a text on both sides from the command line

## Report

To see what one query does, agents write scaffolding. In the 2026-10-26 tracks strength wrote `crates/adtb-testkit/tests/scratch_probe.rs` to open a database from a migration and run a query on the engine and the reference (8 calls, then deleted it); admit added an `ADTB_DBG` `eprintln!` to `write/delta/mod.rs` to see the incremental check a constraint compiles to, and removed it after; app wrote `.adt` files to the scratchpad and ran the app's test binary to learn whether they check. The shell (`adtb FILE -c TEXT`) runs a text on the engine only, needs a database file, and no agent used it.

Fix: one command, for example `adtb-probe --schema FILE [--data FILE] TEXT`, that checks the schema and the text and prints the checker's refusal with its span, or runs the text on the reference and on the engine in memory and prints both results and whether they agree. `--explain` prints the engine's plan, and for a constraint the incremental checks it was admitted with. Name it in `CLAUDE.md`.
