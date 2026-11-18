---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 21
tags: [tooling]
summary: "delete the tools, crates and app the maintainer ruled out in the tooling and crate review"
---

# Feature: Tooling deletions

## Request

The maintainer reviewed every tool in work/tools, every devenv.nix script and every crate on 2026-11-17 and ruled these out: the campaign app ("not mature enough to support an actual app yet"), adtb-bench with bench-run and bench-report, slot-test, workspace-gc, codemap, docs-site, check-reach and its test, feature-check and its test (the regex rule moves to deny.toml), every devenv.nix script but miri, and adtb-server's unused dependency on adtb. code-commits folds into quality-metrics; check moves out of devenv.nix.

## 2026-11-18: resolved

Landed in `refactor!: delete the campaign app, bench crate and tooling the maintainer ruled out`. The regex rule is a cargo-deny ban in deny.toml, its wrapper chain (matchers through tracing-subscriber through loom) closed; check moved to work/tools/check and runs every part. The campaign's five migrations stay as the stored-hash corpus under crates/adtb/src/catalog/hash_corpus/. Lost coverage is filed as release-upgrade-coverage.
