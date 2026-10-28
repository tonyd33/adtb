---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 898
summary: "docs site wraps mdBook with unnecessary staging, watchers and ignore overrides"
branch: docs-native
tags:
  - quality
  - tooling
---

# Docs site native integration

## Report

User rejected tools/docs-site/site.py quality. Lead approved a functional repair
without challenging why a Python polling supervisor, symlink source view,
generated configuration/assets and book-local ignore override existed. Tests
proved the layers worked, not that mdBook needed them. The temporary-file atomic
publication itself is conventional, but was part of an unnecessary pipeline.

## 2026-10-28: corrective track and review bar

Fresh docs-native workspace /home/tony/code/adtb-docs-native on d157a20d, brief
brief-docs-native.md. Prototype proved native mdBook preprocessor Chapter insertion.
Approved routine baseline: checked-in book.toml, docs source directly, minimal
checked-in SUMMARY roots/landing, stateless supported README navigation preprocessor,
direct JS asset path, native mdBook build/serve/watch/lifecycle, separate checks.
Remove obsolete staging/generated config/copied sources/assets/custom watcher/
supervisor/ignore override/temp publication path. No new dependency or hosting.
Preserve unchanged authored manual/spec/design sources, navigation order/live
updates/search/highlighting and actualHTTP checks/malformed recovery/shutdown.

Lead workflow now requires architecture comparison with native configuration/
extension points and fresh review challenging each customlayer's unmet requirement;
green tests are not a design justification. Fresh independent architecture AND
correctness review, scoped docs/syntax/link/HTTP checks plus main verification gate
landing. Retain exact tested/base IDs, stable inputs, exits/logs and remaining limits.
