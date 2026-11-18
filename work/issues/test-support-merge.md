---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 27
tags: [tooling, test]
summary: "merge adtb-testkit, adtb-harness and adtb-isolation; fold codegen-tests and manual; simplify heap-cap"
---

# Feature: Test support merge

## Request

Maintainer rulings of 2026-11-17 (work/briefs/decisions-tooling-crates-2026-11-17.md, Crates): testkit, harness and isolation merge into one engine-free crate; simkit stays engine-side; codegen-tests and manual fold into one crate with one build.rs; heap-cap simplified. The goal beyond this is one test-support crate once the engine tests leave adtb (with the adtb split).

## 2026-11-18: resolved

Landed in `refactor!: merge adtb-harness and adtb-isolation into adtb-testkit, adtb-manual into adtb-codegen-tests`. adtb-testkit stays engine-free; its sim-backed modules sit behind a `sim` feature that only adtb, adtb-sim, adtb-simkit and adtb-codegen-tests enable. codegen-tests and manual merged with each other (generated code needs adtb as a normal dependency). heap-cap: current_cap cut, 874 to 857 lines; both guard tests kept. Test names identical before and after.
