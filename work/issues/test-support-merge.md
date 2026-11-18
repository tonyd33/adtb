---
created-at: 2026-11-18
status: open
kind: feature
priority: 27
tags: [tooling, test]
summary: "merge adtb-testkit, adtb-harness and adtb-isolation; fold codegen-tests and manual; simplify heap-cap"
---

# Feature: Test support merge

## Request

Maintainer rulings of 2026-11-17 (work/briefs/decisions-tooling-crates-2026-11-17.md, Crates): testkit, harness and isolation merge into one engine-free crate; simkit stays engine-side; codegen-tests and manual fold into one crate with one build.rs; heap-cap simplified. The goal beyond this is one test-support crate once the engine tests leave adtb (with the adtb split).
