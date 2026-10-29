---
created-at: 2026-10-28
status: in-progress
branch: lint-audit
kind: feature
priority: 120
summary: "about 87 lint expects and 7 size-allowlist entries stand in the source, and each one is a precedent other agents copy"
tags:
  - quality
---

# Feature: minimize lint suppressions

## Report

Main 28bacb53 has about 87 `#[expect(..., reason = ...)]` attributes: dead_code about 38, clippy::too_many_lines 24, clippy::disallowed_methods 10, clippy::print_stderr and print_stdout 8, clippy::cognitive_complexity 5, and a few others; adtb-sim holds about 33 and adtb-lang about 13. work/tools/size-allowlist exempts 7 files from the source-size limit. Agents read these as license to suppress. Keep only suppressions where the rule is wrong for that spot, each with a reason stating what would remove it, and stop new ones spreading.

## 2026-10-28: audit dispatched

A read-only auditor (workspace lint-audit, no builds) classifies every suppression as remove, narrow or keep, groups fixes into batches by the files running tracks own, and recommends one prevention mechanism. Report: work/briefs/report-lint-audit.md (local); the lead copies the verdicts and batches here before dispatching fix tracks. Local brief: work/briefs/brief-lint-audit.md.
