---
created-at: 2026-11-01
status: resolved
closed: 2026-11-01
kind: bug
priority: 400
summary: "workspace-gc clean can delete retained target evidence"
tags: [quality]
---

# workspace-gc clean can delete retained target evidence

## Report

`work/tools/workspace-gc --clean` passes whole target roots to `cargo clean`.
These roots also hold retained data, logs, provenance and standalone tools.
A Cargo cache tag does not establish ownership of those siblings. Forgotten
workspaces also bypass the normal idle and active-process eligibility checks.
The cause is a separate cleanup entry path that bypasses the existing narrow
`--clear-build` gate. Defect class: authority and admission checked per entry path.

Acceptance: every cleanup mode removes only owned debug/tmp contents, preserves
the Cargo lock and all siblings, and enforces existing path, idle, process and
machine-lock checks for known and forgotten workspaces. Disposable-size reporting
excludes retained siblings. Searching scratch fixtures cover both target roots,
cleanup modes and workspace classifications, with valid and missing Cargo tags.
A real whole-target Cargo-clean mutation must fail the retention oracle.

## 2026-11-01: closed

Landed `4d6747bc`. Automatic and explicit cleanup now share the guarded
debug/tmp deletion boundary. Retained siblings are excluded from deletion,
reported cache size and cache-idle age; forgotten workspaces also retain idle
and active-use admission.

Independent source review and the complete slot-test pass. The searching
retention fixtures cover both target roots and tag conditions. Real scratch
whole-target Cargo-clean mutations delete the sentinels and fail the same
retention oracle; a broad workspace-process predicate mutation fails the
explicit same-workspace cleanup case. Existing shellcheck findings match the
baseline, with no introduced diagnostics. The landed tree matches the tested
tree exactly, so no unchanged engine or tool suite was repeated.
