---
created-at: 2026-11-18
status: resolved
closed: 2026-11-18
kind: feature
priority: 20
tags: [tooling]
summary: "the public check and test entry points as an xtask crate, replacing work/tools/check, suite, limited, loom, miri and size-check"
---

# Feature: Xtask check

## Request

Ruled by the maintainer in the tooling review of 2026-11-17 (checks CI needs live in the public repo, held to a much higher standard than work/). The brief is work/briefs/brief-xtask-check-r1.md.

## 2026-11-18: resolved

Landed in `refactor!: replace the check, suite, limited, loom, miri and size-check scripts with an xtask crate`. `cargo xtask test` and `cargo xtask check [REVSET]` are the entry points; CI calls the check. Miri's nightly comes from devenv's `ADTB_NIGHTLY_BIN`. Main's first `cargo xtask check` passed. Not yet exercised: CI's nix build of the new source, a machine without a systemd user manager. The check still calls structure-check, refusal-check, house-check and tree-sitter-check under work/tools until they are rewritten.
