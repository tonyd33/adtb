---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 89
summary: "no command checks a schema and a text and runs it on the reference and the engine side by side, so agents write throwaway tests to try a query"
closed: 2026-10-26
tags:
  - tooling
  - devex
---

# Feature: try a text on both sides from the command line

## Report

To see what one query does, agents write scaffolding. In the 2026-10-26 tracks strength wrote `crates/adtb-testkit/tests/scratch_probe.rs` to open a database from a migration and run a query on the engine and the reference (8 calls, then deleted it); admit added an `ADTB_DBG` `eprintln!` to `write/delta/mod.rs` to see the incremental check a constraint compiles to, and removed it after; app wrote `.adt` files to the scratchpad and ran the app's test binary to learn whether they check. The shell (`adtb FILE -c TEXT`) runs a text on the engine only, needs a database file, and no agent used it.

Fix: one command, for example `adtb-probe --schema FILE [--data FILE] TEXT`, that checks the schema and the text and prints the checker's refusal with its span, or runs the text on the reference and on the engine in memory and prints both results and whether they agree. `--explain` prints the engine's plan, and for a constraint the incremental checks it was admitted with. Name it in `CLAUDE.md`.

## 2026-10-26: done

`crates/adtb-probe` builds `adtb-probe --schema FILE [--data FILE] [--explain] [--reference|--engine] TEXT`, run through `work/tools/adtb-probe` in the test profile. It checks the schema and the text with `adtb-lang` and prints each refusal as `FILE:LINE:COL` with the source line and a caret; else it builds a conformance script (schema, data as `run`, the text as a `query` when it ends in one, the writes before it as a `run`) and runs it through `adtb_conformance::run_reference_in` and `run_engine_in` over the simulator, printing each side's rows or refusal and `agree` or `differ`. `--explain` prints the engine's `explain` text. Open: the incremental checks a constraint is admitted with; the engine exposes them neither through `explain` nor through a public call, so this needs an engine-side surface.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the devex track.
