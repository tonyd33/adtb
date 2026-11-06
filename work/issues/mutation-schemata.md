---
created-at: 2026-11-06
status: open
kind: feature
priority: 520
summary: cargo-mutants rebuilds per mutant (about 140 s); compile every mutant of a diff into one build switched at run time
---

# Feature: Mutation schemata for the vacuity run

## Request

## Proposal

## Open questions

## 2026-11-06

Measured by the vacuity-gate track: 3.5 to 4.5 min per viable mutant, 130 to 150 s of it recompiling adtb, adtb-sim, adtb-simkit, adtb-conformance and the sim binary; a 50-mutant commit about 3 h. work/tools/mutants already compiles hand sites into one build switched by ADTB_MUTANT. Generate the diff's mutants from `cargo mutants --list --in-diff --json` as run-time switched sites in one build, then run each under the machine lock. Receipt with the numbers: the vacuity-gate track's.
