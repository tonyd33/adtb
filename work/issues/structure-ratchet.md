---
created-at: 2026-11-07
status: open
kind: feature
priority: 10
summary: structure-check holds visible fields and cross-module state access at counts that only fall; slot refuses a brief without a Contract section and a track past the quality cadence
---

# Feature: A structure ratchet and the gates that keep restructuring from regressing

## Request

## Proposal

## Open questions

## 2026-11-07

Part of the structural program (maintainer: lack of invariants, modularity and explicit contracts; every other ticket waits). Baselines from the audits of 2026-11-07: 335 crate-visible fields in crates/*/src, 37 in storage/pager.rs, 85 `.meta()` sites in 10 storage files, 356 `write_path()` sites in 23 files.

- `work/tools/structure-check` in `check`, like size-check: per file, fields visible past their module and named cross-module access patterns (`.meta()` outside the pager, `write_path()`, `.xmin`/`.xmax` outside mvcc, `storage::owner(` outside storage), against an allowlist whose counts may only fall. A stale count (actual below allowed) fails too, so it ratchets.
- `slot` refuses a brief that changes code without a `## Contract` section, and refuses a new track while `work/tools/code-commits reviewed..main` lists more than 15 commits.
- The quality review reviews modules (contract, state, invariants, owners, enforcers), not the diff.
