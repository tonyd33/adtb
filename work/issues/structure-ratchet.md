---
created-at: 2026-11-07
status: resolved
kind: feature
priority: 10
summary: structure-check holds visible fields and cross-module state access at counts that only fall; slot refuses a brief without a Contract section and a track past the quality cadence
closed: 2026-11-07
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

## 2026-11-07, landed

Landed in aa84aefe: `structure-check` (fields counted by parsing with syn in crates/adtb-structure; `owner`, `Pager::meta` and `write_path` in clippy's `disallowed-methods`, each allowed site counted by its `#[expect]`; `xmin`/`xmax` a text row until structure-version-owner), `--against REV` in `check --changed` so no count rises across a change, `slot` refusing a brief with no or an empty `## Contract` and a track past 15 code commits since `reviewed`, the quality review rescoped to modules. Gap left: `--against` passes when REV has no allowlist or does not resolve.

The five structural audits of 2026-11-07 (work/briefs/audit-structure-r1-*.md and design-storage-structure-r1-report.md, local) reviewed every module for contract, state, invariants, owners and enforcers, and their findings are this program's tickets; they count as the quality review for `reviewed..main`, and `reviewed` moves to main.

## 2026-11-07: resolved
