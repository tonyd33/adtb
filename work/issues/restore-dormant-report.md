---
created-at: 2026-11-04
status: open
kind: feature
priority: 540
summary: interop.md 6.1 says a restore reports every dormant grant but gives no shape; the restore command returns only the manifest row
---

# Feature: Restore command does not report dormant grants

## Request

interop.md 6.1 says a restore reports every grant it restores dormant, without stating the report's shape. Dump S5.2 builds the list as `Restored.dormant` from a `restore_report_in` that is doc hidden and non-exhaustive; the `restore` command, the surface the spec names, still returns only the manifest row, so `adtb restore` users never see dormant grants. Decide the report's shape in the spec (rows beside the manifest row, or a second result), then return it from the command and make the API public.

## Proposal

## Open questions
