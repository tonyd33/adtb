---
created-at: 2026-11-18
status: open
kind: feature
priority: 260
tags: [test]
summary: weak refusal assertions inside test helpers the TQL rule does not reach
---

# Feature: Refusal helpers

## Request

From the refusal-sites review (work/briefs/review-refusal-sites-r1.md, findings 1 and 2). Test helpers keep weak refusal checks the rule work/briefs/tql/refusal.tql (lint/ once landed) does not reach: backup_tests.rs:158 `is_invalid_naming` (15 callers, including the sweeps at 2233, 2402, 2742); server_commands.rs:843 `quarantined` (Debug substring; also 814, 816, 945); backup_tests.rs:816, 832 `detail.contains("export_roots")`; testkit inhabit.rs `m.why.contains` at 97, 175, 409, 461 and `shown.contains` at 471; api/server/tests.rs:62 `denied` matching `Denied { .. }` (9 callers). Extend the rule to `.contains` on an error's text or `why` field and `..` patterns over a refusal, then assert each exactly; dump_tests.rs `refused_invalid` moved into test_world::api is the helper to share (every detail text is in api/backup.rs `refused`). Also missed by the rule (public-checks review): `e.contains(..)` and `m.why.contains(..)` at analysis/tests.rs:616, 955, evolve_tests.rs:2232, and `let _ = x.unwrap_err();`.
