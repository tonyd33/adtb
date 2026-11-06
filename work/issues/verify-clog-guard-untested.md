---
created-at: 2026-11-06
status: open
kind: bug
priority: 450
summary: "commit_log_damage's guard *p == damage replaced with true survives: no test meets a second damaged page with the commit log quarantined"
tags:
  - class-weak-tests
---

# Test commit_log_damage's guard

## Report

## 2026-11-06

cargo-mutants survivor at crates/adtb/src/api/verify.rs:674:38 (guard `*p == damage` to `true`). Missing test in clog_lost.rs: verify with the commit log quarantined by checksum damage meets a second, different damaged page and must refuse with that page's error.
